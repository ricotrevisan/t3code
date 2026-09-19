/**
 * Versioned contracts for trusted local provider adapter packages.
 *
 * Package registration is server-local and separate from provider instance
 * configuration. Clients may receive these declarative values, but never load
 * the package module itself.
 */
import * as Schema from "effect/Schema";
import { ForwardCompatibleArray, PositiveInt, TrimmedNonEmptyString } from "./baseSchemas.ts";
import {
  ProviderAdapterPackageId,
  ProviderAdapterPackageVersion,
} from "./providerAdapterIdentity.ts";
import { ProviderDriverKind } from "./providerInstance.ts";

export {
  ProviderAdapterPackageId,
  ProviderAdapterPackageReference,
  ProviderAdapterPackageVersion,
} from "./providerAdapterIdentity.ts";

export const T3_PROVIDER_ADAPTER_PROTOCOL_VERSION = 1 as const;
export const T3_PROVIDER_ADAPTER_HOST_PROTOCOL_VERSION = 2 as const;
export const T3_PROVIDER_ADAPTER_SUPPORTED_HOST_PROTOCOL_VERSIONS = [
  1,
  T3_PROVIDER_ADAPTER_HOST_PROTOCOL_VERSION,
] as const;

export const ProviderAdapterCapability = Schema.Literals([
  "session.resume",
  "turn.steer",
  "turn.follow-up",
  "turn.interrupt",
  "input.attachments",
  "request.approval",
  "request.structured-input",
  "model.discovery",
  "model.switch",
  "reasoning.selection",
  "stream.reasoning",
  "stream.tool-lifecycle",
  "stream.usage",
  "stream.context",
  "stream.subagents",
  "conversation.rollback",
  "feedback.upload",
]);
export type ProviderAdapterCapability = typeof ProviderAdapterCapability.Type;

export const ProviderAdapterCapabilitySet = Schema.Array(ProviderAdapterCapability).check(
  Schema.isUnique(),
);
export type ProviderAdapterCapabilitySet = typeof ProviderAdapterCapabilitySet.Type;

export const ProviderAdapterProtocolCapabilitiesV1 = Schema.Struct({
  protocolVersion: Schema.Literal(T3_PROVIDER_ADAPTER_PROTOCOL_VERSION),
  features: ProviderAdapterCapabilitySet,
});
export type ProviderAdapterProtocolCapabilitiesV1 =
  typeof ProviderAdapterProtocolCapabilitiesV1.Type;

export const ProviderAdapterTransportV1 = Schema.Struct({
  kind: Schema.Literal("supervised-stdio"),
  protocol: Schema.Literals(["jsonl-rpc", "acp-v1"]),
  sessionConcurrency: Schema.Literals(["one-per-process", "multiplexed"]),
});
export type ProviderAdapterTransportV1 = typeof ProviderAdapterTransportV1.Type;

export const ProviderAdapterHostProtocolRange = Schema.Struct({
  minimum: PositiveInt,
  maximum: PositiveInt,
});
export type ProviderAdapterHostProtocolRange = typeof ProviderAdapterHostProtocolRange.Type;

/** JSON-serializable schema metadata clients may use to render package configuration. */
export const ProviderAdapterConfigSchema = Schema.Record(Schema.String, Schema.Json);
export type ProviderAdapterConfigSchema = typeof ProviderAdapterConfigSchema.Type;

/**
 * Declarative package metadata. `configSchema` is JSON-Schema-shaped data;
 * executable code remains in the trusted server-side package module.
 */
export const ProviderAdapterManifestV1 = Schema.Struct({
  protocolVersion: Schema.Literal(T3_PROVIDER_ADAPTER_PROTOCOL_VERSION),
  id: ProviderAdapterPackageId,
  version: ProviderAdapterPackageVersion,
  driver: ProviderDriverKind,
  displayName: TrimmedNonEmptyString,
  hostProtocol: ProviderAdapterHostProtocolRange,
  transport: ProviderAdapterTransportV1,
  capabilities: ProviderAdapterCapabilitySet,
  configSchema: ProviderAdapterConfigSchema,
});
export type ProviderAdapterManifestV1 = typeof ProviderAdapterManifestV1.Type;

/**
 * A server may load adapter protocol versions this client does not know yet.
 * Keep every V1 entry it can decode instead of failing the whole config.
 */
export const ProviderAdapterManifestCatalog = ForwardCompatibleArray(ProviderAdapterManifestV1);
export type ProviderAdapterManifestCatalog = typeof ProviderAdapterManifestCatalog.Type;

export const isProviderAdapterManifestCompatible = (
  manifest: ProviderAdapterManifestV1,
  supportedHostProtocolVersions:
    | number
    | ReadonlyArray<number> = T3_PROVIDER_ADAPTER_SUPPORTED_HOST_PROTOCOL_VERSIONS,
): boolean => {
  if (manifest.hostProtocol.minimum > manifest.hostProtocol.maximum) return false;

  const versions = Array.isArray(supportedHostProtocolVersions)
    ? supportedHostProtocolVersions
    : [supportedHostProtocolVersions];
  return versions.some(
    (version) =>
      manifest.hostProtocol.minimum <= version && manifest.hostProtocol.maximum >= version,
  );
};
