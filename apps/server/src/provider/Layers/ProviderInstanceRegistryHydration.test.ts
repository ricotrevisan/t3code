import { describe, expect, it } from "vite-plus/test";
import {
  DEFAULT_SERVER_SETTINGS,
  ProviderAdapterManifestV1,
  ProviderAdapterPackageId,
  ProviderAdapterPackageVersion,
  ProviderDriverKind,
  ProviderInstanceId,
  type ServerSettings,
} from "@t3tools/contracts";
import * as Schema from "effect/Schema";

import { makeProviderAdapterManifestCatalog } from "../ProviderAdapterManifestCatalog.ts";
import { PRIME_PROVIDER_ADAPTER_PACKAGE_REFERENCE } from "../FirstPartyProviderAdapters.ts";
import { deriveProviderInstanceConfigMap } from "./ProviderInstanceRegistryHydration.ts";

const decodeProviderAdapterManifest = Schema.decodeUnknownSync(ProviderAdapterManifestV1);

const withProviderInstances = (
  providerInstances: ServerSettings["providerInstances"],
): ServerSettings => ({
  ...DEFAULT_SERVER_SETTINGS,
  providerInstances,
});

describe("provider adapter manifest catalog hydration", () => {
  it("includes compiled and successfully loaded manifests without registrations", () => {
    const trustedLocal = decodeProviderAdapterManifest({
      protocolVersion: 1,
      id: "aaa-external",
      version: "2.0.0",
      driver: "externalHarness",
      displayName: "External Harness",
      hostProtocol: { minimum: 1, maximum: 1 },
      transport: {
        kind: "supervised-stdio",
        protocol: "jsonl-rpc",
        sessionConcurrency: "one-per-process",
      },
      capabilities: ["session.resume"],
      configSchema: { type: "object", properties: {} },
      modulePath: "/must/not/cross/the/wire.mjs",
    });

    const catalog = makeProviderAdapterManifestCatalog([trustedLocal, trustedLocal]);

    expect(catalog.map(({ id, version }) => `${id}@${version}`)).toEqual([
      "aaa-external@2.0.0",
      "deepseek-harness-acp@1.0.0",
      "pi-rpc@1.0.0",
      "prime-rpc@1.0.0",
    ]);
    expect(JSON.stringify(catalog)).not.toContain("modulePath");
  });
});

describe("deriveProviderInstanceConfigMap", () => {
  it("pins the legacy Prime instance to the compiled first-party package", () => {
    const derived = deriveProviderInstanceConfigMap(DEFAULT_SERVER_SETTINGS);

    expect(derived[ProviderInstanceId.make("primeAgent")]?.adapterPackage).toEqual(
      PRIME_PROVIDER_ADAPTER_PACKAGE_REFERENCE,
    );
    expect(derived[ProviderInstanceId.make("codex")]?.adapterPackage).toBeUndefined();
  });

  it("pins an explicit unpinned secondary Prime instance", () => {
    const instanceId = ProviderInstanceId.make("prime_secondary");
    const config = {
      driver: ProviderDriverKind.make("primeAgent"),
      enabled: false,
      config: { binaryPath: "custom-prime", launchArgs: "--debug" },
    };
    const derived = deriveProviderInstanceConfigMap(
      withProviderInstances({ [instanceId]: config }),
    );

    expect(derived[instanceId]).toEqual({
      ...config,
      adapterPackage: PRIME_PROVIDER_ADAPTER_PACKAGE_REFERENCE,
    });
  });

  it("preserves an explicit stale Prime package reference for fail-closed resolution", () => {
    const instanceId = ProviderInstanceId.make("primeAgent");
    const stalePackage = {
      id: ProviderAdapterPackageId.make("prime-rpc"),
      version: ProviderAdapterPackageVersion.make("0.9.0"),
      protocolVersion: 1,
    };
    const config = {
      driver: ProviderDriverKind.make("primeAgent"),
      adapterPackage: stalePackage,
      enabled: false,
      config: {},
    };
    const derived = deriveProviderInstanceConfigMap(
      withProviderInstances({ [instanceId]: config }),
    );

    expect(derived[instanceId]).toEqual(config);
    expect(derived[instanceId]?.adapterPackage).toEqual(stalePackage);
  });
});
