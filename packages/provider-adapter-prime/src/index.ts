import { PRIME_RUNTIME_MODES } from "./runtimeModes.ts";
import {
  ProviderAdapterPackageId,
  ProviderAdapterPackageVersion,
  ProviderDriverKind,
  type ProviderAdapterManifestV1,
  type ProviderAdapterPackageReference,
} from "@t3tools/contracts";
import {
  defineProviderAdapterV1,
  type ProviderAdapterHostV2,
  type ProviderAdapterPackageV1,
} from "@t3tools/provider-adapter";
import * as Effect from "effect/Effect";
import * as Schema from "effect/Schema";

import {
  makePrimeAdapter,
  PRIME_ADAPTER_PROTOCOL_CAPABILITIES,
  type PrimeAdapterConfig,
} from "./PrimeAdapter.ts";
import { makePrimeProviderSnapshot } from "./PrimeProvider.ts";

export const PRIME_PROVIDER_ADAPTER_MANIFEST: ProviderAdapterManifestV1 = {
  protocolVersion: 1,
  id: ProviderAdapterPackageId.make("prime-rpc"),
  version: ProviderAdapterPackageVersion.make("1.0.0"),
  driver: ProviderDriverKind.make("primeAgent"),
  displayName: "Prime Agent",
  hostProtocol: { minimum: 2, maximum: 2 },
  transport: {
    kind: "supervised-stdio",
    protocol: "jsonl-rpc",
    sessionConcurrency: "one-per-process",
  },
  capabilities: [...PRIME_ADAPTER_PROTOCOL_CAPABILITIES.features],
  runtimeModes: PRIME_RUNTIME_MODES,
  configSchema: {
    type: "object",
    additionalProperties: false,
    properties: {
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

export const PrimeProviderAdapterConfig = Schema.Struct({
  binaryPath: Schema.String.pipe(Schema.withDecodingDefault(Effect.succeed("prime-agent"))),
  launchArgs: Schema.String.pipe(Schema.withDecodingDefault(Effect.succeed(""))),
});
export type PrimeProviderAdapterConfig = typeof PrimeProviderAdapterConfig.Type;

export const PRIME_PROVIDER_ADAPTER_DEFAULT_CONFIG = {
  binaryPath: "prime-agent",
  launchArgs: "",
} satisfies PrimeProviderAdapterConfig;

export const PRIME_PROVIDER_ADAPTER_PACKAGE: ProviderAdapterPackageV1<
  PrimeProviderAdapterConfig,
  ProviderAdapterHostV2
> = defineProviderAdapterV1<PrimeProviderAdapterConfig, ProviderAdapterHostV2>({
  manifest: PRIME_PROVIDER_ADAPTER_MANIFEST,
  storageKey: "prime-agent",
  configSchema: PrimeProviderAdapterConfig,
  defaultConfig: () => PRIME_PROVIDER_ADAPTER_DEFAULT_CONFIG,
  create: (input, host) =>
    Effect.gen(function* () {
      const snapshot = yield* makePrimeProviderSnapshot(input, host, {
        packageId: PRIME_PROVIDER_ADAPTER_MANIFEST.id,
        packageVersion: PRIME_PROVIDER_ADAPTER_MANIFEST.version,
        manifestConfigSchema: PRIME_PROVIDER_ADAPTER_MANIFEST.configSchema,
        capabilities: PRIME_ADAPTER_PROTOCOL_CAPABILITIES,
      });
      const adapter = yield* makePrimeAdapter(
        input.config satisfies PrimeAdapterConfig,
        input,
        host,
      );
      return { snapshot, adapter };
    }),
});

export { makePrimeAdapter, PRIME_ADAPTER_PROTOCOL_CAPABILITIES } from "./PrimeAdapter.ts";
export type { PrimeAdapterConfig } from "./PrimeAdapter.ts";
export {
  checkPrimeProviderStatus,
  makePrimeProviderSnapshot,
  PRIME_PRESENTATION,
} from "./PrimeProvider.ts";
export * from "./primeApprovalExtension.ts";
export * from "./primeModels.ts";
export * from "./primeOpenRouterCatalogExtension.ts";
export { makePrimeRpcClient, PrimeRpcError } from "./PrimeRpcClient.ts";
