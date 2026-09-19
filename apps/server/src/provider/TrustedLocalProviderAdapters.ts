/**
 * Trusted-local external provider adapter package loader.
 *
 * The installed package registry is a server-local file, separate from
 * `ServerSettings.providerInstances`. Loading happens only in the server
 * process. Remote clients receive normalized provider snapshots and events,
 * never module paths or executable package code.
 */
import type { ProviderAdapterPackageV1 } from "@t3tools/provider-adapter";
import {
  isProviderAdapterManifestCompatible,
  ProviderAdapterManifestV1,
  ProviderAdapterPackageId,
  ProviderAdapterPackageVersion,
  ProviderDriverKind,
  type ProviderAdapterManifestV1 as ProviderAdapterManifest,
  type ProviderAdapterPackageReference,
  type ProviderDriverKind as ProviderDriverKindType,
} from "@t3tools/contracts";
import * as Effect from "effect/Effect";
import * as FileSystem from "effect/FileSystem";
import * as Path from "effect/Path";
import * as Schema from "effect/Schema";

import {
  type ExternalProviderDriverEnv,
  makeExternalProviderDriver,
} from "./ExternalProviderDriver.ts";
import type { AnyProviderDriver } from "./ProviderDriver.ts";

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

const ExternalProviderAdapterPackageV1Schema = Schema.Struct({
  manifest: ProviderAdapterManifestV1,
  storageKey: Schema.optional(Schema.String),
  configSchema: Schema.Unknown,
  defaultConfig: Schema.Unknown,
  create: Schema.Unknown,
});

export type ExternalProviderAdapterPackageV1<Config = unknown> = ProviderAdapterPackageV1<Config>;

export interface ProviderAdapterPackageDiagnostic {
  readonly registration: ProviderAdapterPackageRegistration | undefined;
  readonly code:
    | "registry-invalid"
    | "module-load-failed"
    | "package-invalid"
    | "package-incompatible"
    | "package-mismatch"
    | "package-conflict"
    | "driver-conflict";
  readonly detail: string;
}

export interface LoadedTrustedProviderAdapters {
  readonly drivers: ReadonlyArray<AnyProviderDriver<ExternalProviderDriverEnv>>;
  readonly manifests: ReadonlyArray<ProviderAdapterManifest>;
  readonly unavailableDriverReasons: ReadonlyMap<ProviderDriverKindType, string>;
  readonly diagnostics: ReadonlyArray<ProviderAdapterPackageDiagnostic>;
}

interface LoadTrustedProviderAdapterPackagesInput {
  readonly registryPath: string;
  readonly reservedDriverKinds: ReadonlySet<ProviderDriverKindType>;
  readonly reservedPackageReferences?: ReadonlyArray<ProviderAdapterPackageReference>;
  readonly loadModule?: (moduleUrl: URL) => Promise<unknown>;
}

const decodeRegistry = Schema.decodeUnknownEffect(
  Schema.fromJsonString(TrustedProviderAdapterRegistryFileV1),
);
const decodePackage = Schema.decodeUnknownEffect(ExternalProviderAdapterPackageV1Schema);

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const packageIdentity = (
  reference: Pick<ProviderAdapterPackageReference, "id" | "version" | "protocolVersion">,
): string => `${reference.id}\u0000${reference.version}\u0000${reference.protocolVersion}`;

const formatPackageIdentity = (
  reference: Pick<ProviderAdapterPackageReference, "id" | "version" | "protocolVersion">,
): string => `${reference.id}@${reference.version} (protocol ${reference.protocolVersion})`;

const isProviderAdapterPackage = (value: {
  readonly manifest: ProviderAdapterManifest;
  readonly configSchema: unknown;
  readonly defaultConfig: unknown;
  readonly create: unknown;
}): value is ProviderAdapterPackageV1<unknown> =>
  Schema.isSchema(value.configSchema) &&
  typeof value.defaultConfig === "function" &&
  typeof value.create === "function";

const emptyResult = (
  diagnostics: ReadonlyArray<ProviderAdapterPackageDiagnostic> = [],
): LoadedTrustedProviderAdapters => ({
  drivers: [],
  manifests: [],
  unavailableDriverReasons: new Map(),
  diagnostics,
});

export const loadTrustedProviderAdapterPackages = Effect.fn("loadTrustedProviderAdapterPackages")(
  function* (
    input: LoadTrustedProviderAdapterPackagesInput,
  ): Effect.fn.Return<LoadedTrustedProviderAdapters, never, FileSystem.FileSystem | Path.Path> {
    const fileSystem = yield* FileSystem.FileSystem;
    const path = yield* Path.Path;
    const exists = yield* fileSystem
      .exists(input.registryPath)
      .pipe(Effect.orElseSucceed(() => false));
    if (!exists) {
      return emptyResult();
    }

    const registryResult = yield* fileSystem
      .readFileString(input.registryPath)
      .pipe(Effect.flatMap(decodeRegistry), Effect.result);
    if (registryResult._tag === "Failure") {
      return emptyResult([
        {
          registration: undefined,
          code: "registry-invalid",
          detail: registryResult.failure.message ?? String(registryResult.failure),
        },
      ]);
    }

    const drivers: Array<AnyProviderDriver<ExternalProviderDriverEnv>> = [];
    const manifests: Array<ProviderAdapterManifest> = [];
    const unavailableDriverReasons = new Map<ProviderDriverKindType, string>();
    const diagnostics: Array<ProviderAdapterPackageDiagnostic> = [];
    const claimedDrivers = new Set(input.reservedDriverKinds);
    const claimedPackages = new Set<string>();
    const reservedPackageIdentities = new Set(
      (input.reservedPackageReferences ?? []).map(packageIdentity),
    );
    const loadModule = input.loadModule ?? ((url: URL) => import(url.href));

    for (const registration of registryResult.success.packages) {
      if (!registration.enabled) {
        unavailableDriverReasons.set(
          registration.driver,
          `External adapter package '${registration.id}@${registration.version}' is disabled.`,
        );
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
      if (packageResult._tag === "Failure" || !isProviderAdapterPackage(packageResult.success)) {
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
      if (reservedPackageIdentities.has(packageIdentity(manifest))) {
        const detail = `Adapter package '${formatPackageIdentity(manifest)}' conflicts with a compiled package identity. Choose a different package id or version.`;
        diagnostics.push({ registration, code: "package-conflict", detail });
        unavailableDriverReasons.set(registration.driver, detail);
        continue;
      }

      const defaultConfigResult = yield* Effect.try({
        try: () => adapterPackage.defaultConfig(),
        catch: (cause) => `Default configuration factory threw: ${String(cause)}`,
      }).pipe(
        Effect.flatMap(Schema.decodeUnknownEffect(adapterPackage.configSchema)),
        Effect.mapError((cause) => String(cause)),
        Effect.catchDefect((cause) =>
          Effect.fail(`Default configuration validation defected: ${String(cause)}`),
        ),
        Effect.result,
      );
      if (defaultConfigResult._tag === "Failure") {
        const detail = `Adapter '${packageKey}' has an invalid default configuration: ${defaultConfigResult.failure}`;
        diagnostics.push({ registration, code: "package-invalid", detail });
        unavailableDriverReasons.set(registration.driver, detail);
        continue;
      }

      const driver = makeExternalProviderDriver(adapterPackage, defaultConfigResult.success);
      drivers.push(driver);
      manifests.push(manifest);
      claimedDrivers.add(driver.driverKind);
      unavailableDriverReasons.delete(driver.driverKind);
    }

    return { drivers, manifests, unavailableDriverReasons, diagnostics };
  },
);
