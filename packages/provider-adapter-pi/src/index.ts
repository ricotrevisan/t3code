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

import { makePiAdapter, PI_PROVIDER_ADAPTER_CAPABILITIES } from "./PiAdapter.ts";

export const PI_PROVIDER_ADAPTER_MANIFEST: ProviderAdapterManifestV1 = {
  protocolVersion: 1,
  id: ProviderAdapterPackageId.make("pi-rpc"),
  version: ProviderAdapterPackageVersion.make("1.0.0"),
  driver: ProviderDriverKind.make("piRpc"),
  displayName: "Pi",
  hostProtocol: { minimum: 2, maximum: 2 },
  transport: {
    kind: "supervised-stdio",
    protocol: "jsonl-rpc",
    sessionConcurrency: "one-per-process",
  },
  capabilities: [...PI_PROVIDER_ADAPTER_CAPABILITIES.features],
  configSchema: {
    type: "object",
    additionalProperties: false,
    required: ["binaryPath", "args"],
    properties: {
      binaryPath: {
        type: "string",
        default: "pi",
        description: "Path to the Pi executable used by this instance.",
      },
      args: {
        type: "array",
        items: { type: "string" },
        default: [],
        description: "Additional arguments passed to Pi before T3-owned RPC and session flags.",
      },
      cwd: {
        type: "string",
        description: "Default working directory when a session does not provide one.",
      },
      model: {
        type: "string",
        description: "Default provider-qualified model ID, such as anthropic/claude-sonnet-4.",
      },
      thinkingLevel: {
        type: "string",
        description: "Default Pi thinking level.",
      },
    },
  },
};

export const PI_PROVIDER_ADAPTER_PACKAGE_REFERENCE: ProviderAdapterPackageReference = {
  id: PI_PROVIDER_ADAPTER_MANIFEST.id,
  version: PI_PROVIDER_ADAPTER_MANIFEST.version,
  protocolVersion: PI_PROVIDER_ADAPTER_MANIFEST.protocolVersion,
};

export const PiProviderAdapterConfig = Schema.Struct({
  binaryPath: Schema.String.pipe(Schema.withDecodingDefault(Effect.succeed("pi"))),
  args: Schema.Array(Schema.String).pipe(Schema.withDecodingDefault(Effect.succeed([]))),
  cwd: Schema.optional(Schema.String),
  model: Schema.optional(Schema.String),
  thinkingLevel: Schema.optional(Schema.String),
});
export type PiProviderAdapterConfig = typeof PiProviderAdapterConfig.Type;

export const PI_PROVIDER_ADAPTER_DEFAULT_CONFIG = {
  binaryPath: "pi",
  args: [],
} satisfies PiProviderAdapterConfig;

export const PI_PROVIDER_ADAPTER_PACKAGE: ProviderAdapterPackageV1<
  PiProviderAdapterConfig,
  ProviderAdapterHostV2
> = defineProviderAdapterV1<PiProviderAdapterConfig, ProviderAdapterHostV2>({
  manifest: PI_PROVIDER_ADAPTER_MANIFEST,
  configSchema: PiProviderAdapterConfig,
  defaultConfig: () => PI_PROVIDER_ADAPTER_DEFAULT_CONFIG,
  create: (input, host) =>
    Effect.gen(function* () {
      const adapter = yield* makePiAdapter(input.config, input, host);
      return adapter;
    }),
});

export {
  makePiAdapter,
  mapPiModelIdentity,
  PI_PROVIDER_ADAPTER_CAPABILITIES,
} from "./PiAdapter.ts";
