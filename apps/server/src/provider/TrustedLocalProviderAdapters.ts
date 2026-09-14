/**
 * Trusted-local external provider adapter package loader.
 *
 * The installed package registry is a server-local file, separate from
 * `ServerSettings.providerInstances`. Loading happens only in the server
 * process. Remote clients receive normalized provider snapshots and events,
 * never module paths or executable package code.
 */
import {
  isProviderAdapterManifestCompatible,
  ProviderAdapterManifestV1,
  ProviderAdapterProtocolCapabilitiesV1,
  ProviderAdapterPackageId,
  ProviderAdapterPackageVersion,
  ProviderDriverKind,
  type ProviderAdapterManifestV1 as ProviderAdapterManifest,
  type ProviderAdapterPackageReference,
  type ProviderDriverKind as ProviderDriverKindType,
  type ProviderSession,
  type ServerProvider,
} from "@t3tools/contracts";
import * as Effect from "effect/Effect";
import * as FileSystem from "effect/FileSystem";
import * as Path from "effect/Path";
import * as Schema from "effect/Schema";
import * as Stream from "effect/Stream";

import { ProviderDriverError } from "./Errors.ts";
import type { AnyProviderDriver, ProviderInstance } from "./ProviderDriver.ts";

const ProviderAdapterPackageRegistration = Schema.Struct({
  id: ProviderAdapterPackageId,
  version: ProviderAdapterPackageVersion,
  driver: ProviderDriverKind,
  modulePath: Schema.String.check(Schema.isTrimmed(), Schema.isNonEmpty()),
  enabled: Schema.Boolean.pipe(Schema.withDecodingDefault(Effect.succeed(true))),
});
type ProviderAdapterPackageRegistration = typeof ProviderAdapterPackageRegistration.Type;

const TrustedProviderAdapterRegistryFileV1 = Schema.Struct({
  schemaVersion: Schema.Literal(1),
  packages: Schema.Array(ProviderAdapterPackageRegistration),
});

const ExternalProviderAdapterPackageV1 = Schema.Struct({
  manifest: ProviderAdapterManifestV1,
  driver: Schema.Unknown,
});

export interface ExternalProviderAdapterPackageV1<R = never> {
  readonly manifest: ProviderAdapterManifest;
  readonly driver: AnyProviderDriver<R>;
}

export interface ProviderAdapterPackageDiagnostic {
  readonly registration: ProviderAdapterPackageRegistration | undefined;
  readonly code:
    | "registry-invalid"
    | "module-load-failed"
    | "package-invalid"
    | "package-incompatible"
    | "package-mismatch"
    | "driver-conflict";
  readonly detail: string;
}

export interface LoadedTrustedProviderAdapters<R> {
  readonly drivers: ReadonlyArray<AnyProviderDriver<R>>;
  readonly manifests: ReadonlyArray<ProviderAdapterManifest>;
  readonly unavailableDriverReasons: ReadonlyMap<ProviderDriverKindType, string>;
  readonly diagnostics: ReadonlyArray<ProviderAdapterPackageDiagnostic>;
}

interface LoadTrustedProviderAdapterPackagesInput {
  readonly registryPath: string;
  readonly reservedDriverKinds: ReadonlySet<ProviderDriverKindType>;
  readonly loadModule?: (moduleUrl: URL) => Promise<unknown>;
}

const decodeRegistry = Schema.decodeUnknownEffect(
  Schema.fromJsonString(TrustedProviderAdapterRegistryFileV1),
);
const decodePackage = Schema.decodeUnknownEffect(ExternalProviderAdapterPackageV1);
const decodeNegotiatedCapabilities = Schema.decodeUnknownExit(
  ProviderAdapterProtocolCapabilitiesV1,
);

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const isProviderDriver = (value: unknown): value is AnyProviderDriver<unknown> =>
  isRecord(value) &&
  typeof value.driverKind === "string" &&
  isRecord(value.metadata) &&
  typeof value.metadata.displayName === "string" &&
  (isRecord(value.configSchema) || typeof value.configSchema === "function") &&
  typeof value.defaultConfig === "function" &&
  typeof value.create === "function";

const packageReference = (manifest: ProviderAdapterManifest): ProviderAdapterPackageReference => ({
  id: manifest.id,
  version: manifest.version,
  protocolVersion: manifest.protocolVersion,
});

const stampInstance = <R>(
  manifest: ProviderAdapterManifest,
  driver: AnyProviderDriver<R>,
): AnyProviderDriver<R> => ({
  ...driver,
  create: (input) =>
    driver.create(input).pipe(
      Effect.flatMap((instance) => {
        const negotiatedResult = decodeNegotiatedCapabilities(
          instance.adapter.capabilities.protocol,
        );
        if (negotiatedResult._tag === "Failure") {
          return Effect.fail(
            new ProviderDriverError({
              driver: driver.driverKind,
              instanceId: input.instanceId,
              detail:
                "External adapters must negotiate valid Provider Adapter Protocol V1 capabilities.",
            }),
          );
        }
        const negotiated = negotiatedResult.value;
        const declared = new Set(manifest.capabilities);
        const undeclared = negotiated.features.filter((feature) => !declared.has(feature));
        if (undeclared.length > 0) {
          return Effect.fail(
            new ProviderDriverError({
              driver: driver.driverKind,
              instanceId: input.instanceId,
              detail: `Adapter negotiated undeclared capabilities: ${undeclared.join(", ")}.`,
            }),
          );
        }

        const adapterPackage = packageReference(manifest);
        const withPackage = (snapshot: ServerProvider): ServerProvider => ({
          ...snapshot,
          adapterPackage,
        });
        const snapshot: ProviderInstance["snapshot"] = {
          ...instance.snapshot,
          getSnapshot: instance.snapshot.getSnapshot.pipe(Effect.map(withPackage)),
          refresh: instance.snapshot.refresh.pipe(Effect.map(withPackage)),
          streamChanges: instance.snapshot.streamChanges.pipe(Stream.map(withPackage)),
        };
        const withPackageSession = (session: ProviderSession): ProviderSession => ({
          ...session,
          adapterPackage,
        });
        const adapter: ProviderInstance["adapter"] = {
          ...instance.adapter,
          startSession: (input) =>
            instance.adapter.startSession(input).pipe(Effect.map(withPackageSession)),
          listSessions: () =>
            instance.adapter
              .listSessions()
              .pipe(Effect.map((sessions) => sessions.map(withPackageSession))),
        };

        return Effect.succeed({
          ...instance,
          adapterPackage,
          snapshot,
          adapter,
        } satisfies ProviderInstance);
      }),
    ),
});

const emptyResult = <R>(
  diagnostics: ReadonlyArray<ProviderAdapterPackageDiagnostic> = [],
): LoadedTrustedProviderAdapters<R> => ({
  drivers: [],
  manifests: [],
  unavailableDriverReasons: new Map(),
  diagnostics,
});

export const loadTrustedProviderAdapterPackages = Effect.fn("loadTrustedProviderAdapterPackages")(
  function* <R>(
    input: LoadTrustedProviderAdapterPackagesInput,
  ): Effect.fn.Return<LoadedTrustedProviderAdapters<R>, never, FileSystem.FileSystem | Path.Path> {
    const fileSystem = yield* FileSystem.FileSystem;
    const path = yield* Path.Path;
    const exists = yield* fileSystem
      .exists(input.registryPath)
      .pipe(Effect.orElseSucceed(() => false));
    if (!exists) {
      return emptyResult<R>();
    }

    const registryResult = yield* fileSystem
      .readFileString(input.registryPath)
      .pipe(Effect.flatMap(decodeRegistry), Effect.result);
    if (registryResult._tag === "Failure") {
      return emptyResult<R>([
        {
          registration: undefined,
          code: "registry-invalid",
          detail: registryResult.failure.message ?? String(registryResult.failure),
        },
      ]);
    }

    const drivers: Array<AnyProviderDriver<R>> = [];
    const manifests: Array<ProviderAdapterManifest> = [];
    const unavailableDriverReasons = new Map<ProviderDriverKindType, string>();
    const diagnostics: Array<ProviderAdapterPackageDiagnostic> = [];
    const claimedDrivers = new Set(input.reservedDriverKinds);
    const claimedPackages = new Set<string>();
    const loadModule = input.loadModule ?? ((url: URL) => import(url.href));

    for (const registration of registryResult.success.packages) {
      if (!registration.enabled) {
        continue;
      }

      const packageKey = `${registration.id}@${registration.version}`;
      if (claimedPackages.has(packageKey) || claimedDrivers.has(registration.driver)) {
        diagnostics.push({
          registration,
          code: "driver-conflict",
          detail: claimedPackages.has(packageKey)
            ? `Adapter package '${packageKey}' is registered more than once.`
            : `Provider driver '${registration.driver}' is already registered.`,
        });
        if (!input.reservedDriverKinds.has(registration.driver)) {
          unavailableDriverReasons.set(
            registration.driver,
            `External adapter registration conflict for '${registration.driver}'.`,
          );
        }
        continue;
      }
      claimedPackages.add(packageKey);

      const modulePath = path.isAbsolute(registration.modulePath)
        ? registration.modulePath
        : path.resolve(path.dirname(input.registryPath), registration.modulePath);
      const moduleUrlResult = yield* path.toFileUrl(modulePath).pipe(Effect.result);
      if (moduleUrlResult._tag === "Failure") {
        const detail = `Invalid adapter module path '${registration.modulePath}'.`;
        diagnostics.push({ registration, code: "module-load-failed", detail });
        unavailableDriverReasons.set(registration.driver, detail);
        continue;
      }

      const moduleResult = yield* Effect.tryPromise({
        try: () => loadModule(moduleUrlResult.success),
        catch: (cause) => `Adapter module import failed: ${String(cause)}`,
      }).pipe(Effect.result);
      if (moduleResult._tag === "Failure") {
        const detail = `Failed to load adapter module '${registration.modulePath}': ${String(moduleResult.failure)}`;
        diagnostics.push({ registration, code: "module-load-failed", detail });
        unavailableDriverReasons.set(registration.driver, detail);
        continue;
      }

      const loadedModule = moduleResult.success;
      const exported = isRecord(loadedModule)
        ? (loadedModule.default ?? loadedModule.t3ProviderAdapter)
        : undefined;
      const packageResult = yield* decodePackage(exported).pipe(Effect.result);
      if (packageResult._tag === "Failure") {
        const detail = `Adapter module '${registration.modulePath}' does not export a valid V1 package.`;
        diagnostics.push({ registration, code: "package-invalid", detail });
        unavailableDriverReasons.set(registration.driver, detail);
        continue;
      }

      const adapterPackage = packageResult.success;
      const { manifest } = adapterPackage;
      if (
        manifest.id !== registration.id ||
        manifest.version !== registration.version ||
        manifest.driver !== registration.driver
      ) {
        const detail = `Adapter package identity does not match its installed registration '${packageKey}'.`;
        diagnostics.push({ registration, code: "package-mismatch", detail });
        unavailableDriverReasons.set(registration.driver, detail);
        continue;
      }
      if (!isProviderAdapterManifestCompatible(manifest)) {
        const detail = `Adapter '${packageKey}' is incompatible with T3 provider protocol 1.`;
        diagnostics.push({ registration, code: "package-incompatible", detail });
        unavailableDriverReasons.set(registration.driver, detail);
        continue;
      }
      if (
        !isProviderDriver(adapterPackage.driver) ||
        adapterPackage.driver.driverKind !== manifest.driver
      ) {
        const detail = `Adapter '${packageKey}' does not export the declared provider driver '${manifest.driver}'.`;
        diagnostics.push({ registration, code: "package-invalid", detail });
        unavailableDriverReasons.set(registration.driver, detail);
        continue;
      }

      const driver = stampInstance(manifest, adapterPackage.driver as AnyProviderDriver<R>);
      drivers.push(driver);
      manifests.push(manifest);
      claimedDrivers.add(driver.driverKind);
      unavailableDriverReasons.delete(driver.driverKind);
    }

    return { drivers, manifests, unavailableDriverReasons, diagnostics };
  },
);
