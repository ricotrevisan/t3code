/**
 * Safe declarative catalog metadata for provider adapter packages.
 *
 * This module never contains trusted-local registration records or executable
 * module paths. It validates the compiled manifests and gives registries one
 * deterministic package-identity ordering for wire snapshots.
 *
 * @module provider/ProviderAdapterManifestCatalog
 */
import {
  ProviderAdapterManifestCatalog,
  ProviderAdapterManifestV1,
  type ProviderAdapterManifestV1 as ProviderAdapterManifest,
} from "@t3tools/contracts";
import { PI_PROVIDER_ADAPTER_MANIFEST } from "@t3tools/provider-adapter-pi";
import { PRIME_PROVIDER_ADAPTER_MANIFEST } from "@t3tools/provider-adapter-prime";
import * as Schema from "effect/Schema";

import { DEEPSEEK_HARNESS_ADAPTER_MANIFEST } from "./DeepSeekHarnessAdapter.ts";

const decodeManifest = Schema.decodeUnknownSync(ProviderAdapterManifestV1);
const decodeManifestCatalog = Schema.decodeUnknownSync(ProviderAdapterManifestCatalog);

/** Compiled manifests shipped by this server build, validated at startup. */
export const FIRST_PARTY_PROVIDER_ADAPTER_MANIFESTS: ReadonlyArray<ProviderAdapterManifest> = [
  decodeManifest(PRIME_PROVIDER_ADAPTER_MANIFEST),
  decodeManifest(DEEPSEEK_HARNESS_ADAPTER_MANIFEST),
  decodeManifest(PI_PROVIDER_ADAPTER_MANIFEST),
];

const compareString = (left: string, right: string): number =>
  left < right ? -1 : left > right ? 1 : 0;

const comparePackageIdentity = (
  left: ProviderAdapterManifest,
  right: ProviderAdapterManifest,
): number => {
  const idOrder = compareString(left.id, right.id);
  if (idOrder !== 0) return idOrder;

  const versionOrder = compareString(left.version, right.version);
  if (versionOrder !== 0) return versionOrder;

  return left.protocolVersion - right.protocolVersion;
};

const packageIdentity = (manifest: ProviderAdapterManifest): string =>
  `${manifest.id}\u0000${manifest.version}\u0000${manifest.protocolVersion}`;

/** Deduplicate and sort safe manifests by their exact package identity. */
export const normalizeProviderAdapterManifests = (
  manifests: ReadonlyArray<unknown>,
): ReadonlyArray<ProviderAdapterManifest> => {
  const byIdentity = new Map<string, ProviderAdapterManifest>();
  for (const manifest of decodeManifestCatalog(manifests)) {
    const identity = packageIdentity(manifest);
    if (!byIdentity.has(identity)) {
      byIdentity.set(identity, manifest);
    }
  }
  return [...byIdentity.values()].toSorted(comparePackageIdentity);
};

/** Build the startup catalog from compiled and successfully loaded packages. */
export const makeProviderAdapterManifestCatalog = (
  trustedLocalManifests: ReadonlyArray<ProviderAdapterManifest>,
): ReadonlyArray<ProviderAdapterManifest> =>
  normalizeProviderAdapterManifests([
    ...FIRST_PARTY_PROVIDER_ADAPTER_MANIFESTS,
    ...trustedLocalManifests,
  ]);
