import { describe, expect, it } from "vite-plus/test";
import {
  DEFAULT_SERVER_SETTINGS,
  ProviderAdapterPackageId,
  ProviderAdapterPackageVersion,
  ProviderDriverKind,
  ProviderInstanceId,
  type ServerSettings,
} from "@t3tools/contracts";

import { PRIME_PROVIDER_ADAPTER_PACKAGE_REFERENCE } from "../FirstPartyProviderAdapters.ts";
import { deriveProviderInstanceConfigMap } from "./ProviderInstanceRegistryHydration.ts";

const withProviderInstances = (
  providerInstances: ServerSettings["providerInstances"],
): ServerSettings => ({
  ...DEFAULT_SERVER_SETTINGS,
  providerInstances,
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
