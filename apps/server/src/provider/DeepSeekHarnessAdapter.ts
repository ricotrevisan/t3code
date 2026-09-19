/**
 * Compiled first-party package definition for DeepSeek Harness over ACP v1.
 *
 * Authentication and other secrets come from the provider instance environment.
 * The adapter config only controls how the local harness process is launched.
 *
 * @module provider/DeepSeekHarnessAdapter
 */
import {
  ProviderAdapterPackageId,
  ProviderAdapterPackageVersion,
  ProviderDriverKind,
  type ProviderAdapterManifestV1,
  type ProviderAdapterPackageReference,
} from "@t3tools/contracts";
import type { ProviderAdapterPackageV1 } from "@t3tools/provider-adapter";
import { AcpStdioAdapter } from "@t3tools/provider-adapter-acp";
import * as Schema from "effect/Schema";

export const DEEPSEEK_HARNESS_DRIVER_KIND = ProviderDriverKind.make("deepseekHarness");

export const DEEPSEEK_HARNESS_ADAPTER_MANIFEST: ProviderAdapterManifestV1 = {
  protocolVersion: 1,
  id: ProviderAdapterPackageId.make("deepseek-harness-acp"),
  version: ProviderAdapterPackageVersion.make("1.0.0"),
  driver: DEEPSEEK_HARNESS_DRIVER_KIND,
  displayName: "DeepSeek Harness",
  hostProtocol: { minimum: 1, maximum: 1 },
  transport: {
    kind: "supervised-stdio",
    protocol: "acp-v1",
    sessionConcurrency: "multiplexed",
  },
  capabilities: [
    "session.resume",
    "turn.interrupt",
    "request.approval",
    "model.discovery",
    "model.switch",
    "reasoning.selection",
    "stream.reasoning",
    "stream.tool-lifecycle",
    "stream.context",
  ],
  configSchema: {
    type: "object",
    additionalProperties: false,
    required: ["command", "args"],
    properties: {
      command: {
        type: "string",
        default: "dsh",
        description: "Path to the DeepSeek Harness executable.",
      },
      args: {
        type: "array",
        items: { type: "string" },
        default: ["--profile", "acp"],
        description: "Arguments passed to DeepSeek Harness.",
      },
      cwd: {
        type: "string",
        description: "Optional working directory for the DeepSeek Harness process.",
      },
    },
  },
};

export const DeepSeekHarnessAdapterConfig = Schema.Struct({
  command: Schema.String.check(Schema.isMinLength(1)),
  args: Schema.Array(Schema.String),
  cwd: Schema.optionalKey(Schema.String.check(Schema.isMinLength(1))),
});
export type DeepSeekHarnessAdapterConfig = typeof DeepSeekHarnessAdapterConfig.Type;

export const DEEPSEEK_HARNESS_ADAPTER_DEFAULT_CONFIG = {
  command: "dsh",
  args: ["--profile", "acp"],
} satisfies DeepSeekHarnessAdapterConfig;

const deepSeekHarnessAcpPackage = AcpStdioAdapter.defineAcpStdioAdapterV1({
  manifest: DEEPSEEK_HARNESS_ADAPTER_MANIFEST,
  defaultConfig: () => DEEPSEEK_HARNESS_ADAPTER_DEFAULT_CONFIG,
  clientInfo: { name: "T3 Code", version: "0.0.0" },
});

export const DEEPSEEK_HARNESS_ADAPTER_PACKAGE: ProviderAdapterPackageV1<DeepSeekHarnessAdapterConfig> =
  {
    ...deepSeekHarnessAcpPackage,
    configSchema: DeepSeekHarnessAdapterConfig,
    defaultConfig: () => DEEPSEEK_HARNESS_ADAPTER_DEFAULT_CONFIG,
  };

export const DEEPSEEK_HARNESS_ADAPTER_PACKAGE_REFERENCE: ProviderAdapterPackageReference = {
  id: DEEPSEEK_HARNESS_ADAPTER_MANIFEST.id,
  version: DEEPSEEK_HARNESS_ADAPTER_MANIFEST.version,
  protocolVersion: DEEPSEEK_HARNESS_ADAPTER_MANIFEST.protocolVersion,
};
