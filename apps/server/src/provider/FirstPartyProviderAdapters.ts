/**
 * Server-private bridge for compiled first-party provider adapters.
 *
 * These registrations use the same versioned identity and negotiated
 * capability model as trusted-local packages, while retaining access to
 * private server services that are not part of the public V1 package host.
 * They are transitional compiled registrations, not installable packages.
 *
 * @module provider/FirstPartyProviderAdapters
 */
import {
  ProviderAdapterPackageId,
  ProviderAdapterPackageVersion,
  ProviderDriverKind,
  type ProviderAdapterManifestV1,
  type ProviderAdapterPackageReference,
  type ProviderAdapterProtocolCapabilitiesV1,
  type ProviderSession,
  type ServerProvider,
} from "@t3tools/contracts";
import * as Effect from "effect/Effect";
import * as Stream from "effect/Stream";

import { PrimeDriver, type PrimeDriverEnv } from "./Drivers/PrimeDriver.ts";
import { ProviderDriverError } from "./Errors.ts";
import { PRIME_ADAPTER_PROTOCOL_CAPABILITIES } from "./Layers/PrimeAdapter.ts";
import type { AnyProviderDriver, ProviderDriver, ProviderInstance } from "./ProviderDriver.ts";

const PRIME_DRIVER_KIND = ProviderDriverKind.make("primeAgent");

export const PRIME_PROVIDER_ADAPTER_MANIFEST: ProviderAdapterManifestV1 = {
  protocolVersion: 1,
  id: ProviderAdapterPackageId.make("prime-rpc"),
  version: ProviderAdapterPackageVersion.make("1.0.0"),
  driver: PRIME_DRIVER_KIND,
  displayName: "Prime Agent",
  hostProtocol: { minimum: 1, maximum: 1 },
  transport: {
    kind: "supervised-stdio",
    protocol: "jsonl-rpc",
    sessionConcurrency: "one-per-process",
  },
  capabilities: [...PRIME_ADAPTER_PROTOCOL_CAPABILITIES.features],
  configSchema: {
    type: "object",
    additionalProperties: false,
    properties: {
      enabled: {
        type: "boolean",
        default: false,
        description: "Whether this Prime Agent instance is enabled.",
      },
      binaryPath: {
        type: "string",
        default: "prime-agent",
        description: "Path to the Prime Agent binary used by this instance.",
      },
      launchArgs: {
        type: "string",
        default: "",
        description: "Additional CLI arguments passed on session start.",
      },
    },
  },
};

export const PRIME_PROVIDER_ADAPTER_PACKAGE_REFERENCE: ProviderAdapterPackageReference = {
  id: PRIME_PROVIDER_ADAPTER_MANIFEST.id,
  version: PRIME_PROVIDER_ADAPTER_MANIFEST.version,
  protocolVersion: PRIME_PROVIDER_ADAPTER_MANIFEST.protocolVersion,
};

const capabilityMismatch = (
  manifest: ProviderAdapterManifestV1,
  capabilities: ProviderAdapterProtocolCapabilitiesV1 | undefined,
): string | undefined => {
  if (capabilities === undefined) {
    return "Adapter did not negotiate versioned protocol capabilities.";
  }
  if (capabilities.protocolVersion !== manifest.protocolVersion) {
    return `Adapter negotiated protocol ${capabilities.protocolVersion}, but package '${manifest.id}@${manifest.version}' uses protocol ${manifest.protocolVersion}.`;
  }
  const declared = new Set(manifest.capabilities);
  const undeclared = capabilities.features.filter((feature) => !declared.has(feature));
  return undeclared.length === 0
    ? undefined
    : `Adapter negotiated undeclared capabilities: ${undeclared.join(", ")}.`;
};

const withSessionIdentity = (
  session: ProviderSession,
  instance: Pick<ProviderInstance, "instanceId" | "driverKind">,
  adapterPackage: ProviderAdapterPackageReference,
): ProviderSession => ({
  ...session,
  provider: instance.driverKind,
  providerInstanceId: instance.instanceId,
  adapterPackage,
});

/**
 * Attach package semantics to an existing compiled driver without exposing
 * its private Effect services through the public adapter SDK.
 */
export const makeFirstPartyProviderAdapterDriver = <Config, R>(input: {
  readonly manifest: ProviderAdapterManifestV1;
  readonly driver: ProviderDriver<Config, R>;
}): ProviderDriver<Config, R> => {
  const { driver, manifest } = input;
  const adapterPackage: ProviderAdapterPackageReference = {
    id: manifest.id,
    version: manifest.version,
    protocolVersion: manifest.protocolVersion,
  };

  return {
    ...driver,
    adapterPackage,
    create: (createInput) =>
      driver.create(createInput).pipe(
        Effect.flatMap((instance) => {
          if (
            manifest.driver !== driver.driverKind ||
            instance.driverKind !== driver.driverKind ||
            instance.adapter.provider !== driver.driverKind
          ) {
            return Effect.fail(
              new ProviderDriverError({
                driver: driver.driverKind,
                instanceId: createInput.instanceId,
                detail: `First-party adapter package '${manifest.id}@${manifest.version}' does not match driver '${driver.driverKind}'.`,
              }),
            );
          }
          const capabilities = instance.adapter.capabilities.protocol;
          const mismatch = capabilityMismatch(manifest, capabilities);
          if (mismatch !== undefined) {
            return Effect.fail(
              new ProviderDriverError({
                driver: driver.driverKind,
                instanceId: createInput.instanceId,
                detail: mismatch,
              }),
            );
          }

          const stampSession = (session: ProviderSession) =>
            withSessionIdentity(session, instance, adapterPackage);
          const stampSnapshot = (snapshot: ServerProvider): ServerProvider => ({
            ...snapshot,
            instanceId: instance.instanceId,
            driver: instance.driverKind,
            adapterPackage,
            adapterConfigSchema: manifest.configSchema,
            adapterCapabilities: capabilities,
          });

          return Effect.succeed({
            ...instance,
            adapterPackage,
            adapter: {
              ...instance.adapter,
              adapterPackage,
              startSession: (sessionInput) =>
                instance.adapter.startSession(sessionInput).pipe(Effect.map(stampSession)),
              listSessions: () =>
                instance.adapter
                  .listSessions()
                  .pipe(Effect.map((sessions) => sessions.map(stampSession))),
            },
            snapshot: {
              ...instance.snapshot,
              getSnapshot: instance.snapshot.getSnapshot.pipe(Effect.map(stampSnapshot)),
              refresh: instance.snapshot.refresh.pipe(Effect.map(stampSnapshot)),
              streamChanges: instance.snapshot.streamChanges.pipe(Stream.map(stampSnapshot)),
            },
          } satisfies ProviderInstance);
        }),
      ),
  };
};

export type FirstPartyProviderAdaptersEnv = PrimeDriverEnv;

/** Compiled, server-private package registrations shipped by this build. */
export const FIRST_PARTY_PROVIDER_ADAPTER_DRIVERS: ReadonlyArray<
  AnyProviderDriver<FirstPartyProviderAdaptersEnv>
> = [
  makeFirstPartyProviderAdapterDriver({
    manifest: PRIME_PROVIDER_ADAPTER_MANIFEST,
    driver: PrimeDriver,
  }),
];
