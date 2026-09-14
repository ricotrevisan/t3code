/**
 * Versioned contracts for trusted local provider adapter packages.
 *
 * Package registration is server-local and separate from provider instance
 * configuration. Clients may receive these declarative values, but never load
 * the package module itself.
 */
import * as Schema from "effect/Schema";
import { PositiveInt, TrimmedNonEmptyString } from "./baseSchemas.ts";
import { ProviderDriverKind } from "./providerInstance.ts";

export const T3_PROVIDER_ADAPTER_PROTOCOL_VERSION = 1 as const;

const AdapterSlug = TrimmedNonEmptyString.check(
  Schema.isMaxLength(64),
  Schema.isPattern(/^[a-zA-Z][a-zA-Z0-9_-]*$/),
);

export const ProviderAdapterPackageId = AdapterSlug.pipe(Schema.brand("ProviderAdapterPackageId"));
export type ProviderAdapterPackageId = typeof ProviderAdapterPackageId.Type;

export const ProviderAdapterPackageVersion = TrimmedNonEmptyString.check(
  Schema.isMaxLength(128),
  Schema.isPattern(/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/),
).pipe(Schema.brand("ProviderAdapterPackageVersion"));
export type ProviderAdapterPackageVersion = typeof ProviderAdapterPackageVersion.Type;

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

export const ProviderAdapterPackageReference = Schema.Struct({
  id: ProviderAdapterPackageId,
  version: ProviderAdapterPackageVersion,
  protocolVersion: PositiveInt,
});
export type ProviderAdapterPackageReference = typeof ProviderAdapterPackageReference.Type;

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
  configSchema: Schema.Record(Schema.String, Schema.Unknown),
});
export type ProviderAdapterManifestV1 = typeof ProviderAdapterManifestV1.Type;

export const isProviderAdapterManifestCompatible = (
  manifest: ProviderAdapterManifestV1,
  hostProtocolVersion = T3_PROVIDER_ADAPTER_PROTOCOL_VERSION,
): boolean =>
  manifest.hostProtocol.minimum <= manifest.hostProtocol.maximum &&
  manifest.hostProtocol.minimum <= hostProtocolVersion &&
  manifest.hostProtocol.maximum >= hostProtocolVersion;
