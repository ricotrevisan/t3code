/** Stable identity shared by adapter packages, configured instances, and sessions. */
import * as Schema from "effect/Schema";
import { PositiveInt, TrimmedNonEmptyString } from "./baseSchemas.ts";

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

export const ProviderAdapterPackageReference = Schema.Struct({
  id: ProviderAdapterPackageId,
  version: ProviderAdapterPackageVersion,
  protocolVersion: PositiveInt,
});
export type ProviderAdapterPackageReference = typeof ProviderAdapterPackageReference.Type;
