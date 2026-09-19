import {
  ProviderAdapterPackageId,
  ProviderAdapterPackageVersion,
  ProviderDriverKind,
  ProviderInstanceId,
  type ProviderAdapterManifestV1,
  type ServerProvider,
} from "@t3tools/contracts";
import { describe, expect, it } from "vite-plus/test";

import {
  addableAdapterManifests,
  adapterManifestSelectionKey,
  adapterPackageIdentity,
  buildProviderInstanceConfig,
  defaultAdapterConfig,
  manifestPackageReference,
  nextAdapterConfigWithFieldValue,
  normalizeAdapterConfigSchema,
  resolveAdapterConfigSchema,
  resolveAdapterSelection,
  selectionKeyForBuiltInDriver,
  validateAdapterConfig,
} from "./providerAdapterConfig";

const schema = {
  type: "object",
  required: ["command", "retries"],
  properties: {
    command: { type: "string", title: "Command", default: "agent", format: "password" },
    enabled: { type: "boolean", default: false },
    retries: { type: "integer", default: 2 },
    temperature: { type: "number" },
    mode: { type: "string", enum: ["fast", "safe"], default: "safe" },
    args: { type: "array", items: { type: "string" }, default: ["--acp"] },
    nested: { type: "object", properties: { token: { type: "string" } } },
  },
} as const;

const manifest = {
  protocolVersion: 1,
  id: ProviderAdapterPackageId.make("deepseek-harness-acp"),
  version: ProviderAdapterPackageVersion.make("1.0.0"),
  driver: ProviderDriverKind.make("deepseekHarness"),
  displayName: "DeepSeek Harness",
  hostProtocol: { minimum: 1, maximum: 1 },
  transport: {
    kind: "supervised-stdio",
    protocol: "acp-v1",
    sessionConcurrency: "multiplexed",
  },
  capabilities: [],
  configSchema: schema,
} satisfies ProviderAdapterManifestV1;

function liveProvider(adapterPackage = manifestPackageReference(manifest)): ServerProvider {
  return {
    instanceId: ProviderInstanceId.make("deepseek"),
    driver: manifest.driver,
    adapterPackage,
    adapterConfigSchema: schema,
    enabled: true,
    installed: true,
    version: "1.0.0",
    status: "ready",
    auth: { status: "unknown" },
    checkedAt: "2026-08-01T00:00:00.000Z",
    models: [],
    slashCommands: [],
    skills: [],
  };
}

describe("external provider adapter config", () => {
  it("normalizes supported fields, ignores password presentation, and exposes unsupported fields", () => {
    const normalized = normalizeAdapterConfigSchema(schema);

    expect(normalized.unsupportedRoot).toBe(false);
    expect(normalized.fields.map(({ key, kind, required }) => ({ key, kind, required }))).toEqual([
      { key: "command", kind: "string", required: true },
      { key: "enabled", kind: "boolean", required: false },
      { key: "retries", kind: "integer", required: true },
      { key: "temperature", kind: "number", required: false },
      { key: "mode", kind: "string-enum", required: false },
      { key: "args", kind: "string-array", required: false },
      { key: "nested", kind: "unsupported", required: false },
    ]);
  });

  it("creates a catalog instance pinned to the exact package identity", () => {
    const config = defaultAdapterConfig(normalizeAdapterConfigSchema(manifest.configSchema));
    expect(
      buildProviderInstanceConfig({
        driver: manifest.driver,
        manifest,
        displayName: "DeepSeek Work",
        ...(config === undefined ? {} : { config }),
      }),
    ).toEqual({
      driver: ProviderDriverKind.make("deepseekHarness"),
      adapterPackage: {
        id: ProviderAdapterPackageId.make("deepseek-harness-acp"),
        version: ProviderAdapterPackageVersion.make("1.0.0"),
        protocolVersion: 1,
      },
      displayName: "DeepSeek Work",
      enabled: true,
      config: {
        command: "agent",
        enabled: false,
        retries: 2,
        mode: "safe",
        args: ["--acp"],
      },
    });
  });

  it("derives new-instance config from supported schema defaults", () => {
    expect(defaultAdapterConfig(normalizeAdapterConfigSchema(schema))).toEqual({
      command: "agent",
      enabled: false,
      retries: 2,
      mode: "safe",
      args: ["--acp"],
    });
  });

  it("validates missing required fields and invalid number and array values", () => {
    const normalized = normalizeAdapterConfigSchema(schema);
    expect(validateAdapterConfig(normalized, { retries: 2.5, args: "--acp" })).toEqual({
      command: "Command is required.",
      retries: "Retries has an invalid value.",
      args: "Args has an invalid value.",
    });
  });

  it("preserves unknown and unsupported nested config while repairing a supported value", () => {
    const normalized = normalizeAdapterConfigSchema(schema);
    const retries = normalized.fields.find((field) => field.key === "retries")!;
    expect(
      nextAdapterConfigWithFieldValue(
        { forkOwned: true, nested: { token: "preserve" }, retries: "bad" },
        retries,
        3,
      ),
    ).toEqual({ forkOwned: true, nested: { token: "preserve" }, retries: 3 });
  });

  it("merges a known-driver package into its built-in choice without losing identity", () => {
    const prime = {
      ...manifest,
      id: ProviderAdapterPackageId.make("prime-rpc"),
      driver: ProviderDriverKind.make("primeAgent"),
    };
    const key = selectionKeyForBuiltInDriver(prime.driver, [prime, manifest]);
    const selection = resolveAdapterSelection(key, [prime.driver], [prime, manifest]);

    expect(key).toBe(adapterManifestSelectionKey(prime));
    expect(selection).toEqual({ kind: "manifest", manifest: prime });
    expect(
      resolveAdapterSelection("built-in:primeAgent", [prime.driver], [prime, manifest]),
    ).toEqual({ kind: "built-in", driver: prime.driver, manifest: prime });
    expect(
      buildProviderInstanceConfig({
        driver: prime.driver,
        ...(selection.kind === "manifest" ? { manifest: selection.manifest } : {}),
      }).adapterPackage,
    ).toEqual({
      id: ProviderAdapterPackageId.make("prime-rpc"),
      version: ProviderAdapterPackageVersion.make("1.0.0"),
      protocolVersion: 1,
    });
  });

  it("changes form identity when the selected package version changes", () => {
    const first = manifestPackageReference(manifest);
    const second = {
      ...first,
      version: ProviderAdapterPackageVersion.make("2.0.0"),
    };

    expect(adapterPackageIdentity(first)).toBe("deepseek-harness-acp:1.0.0:1");
    expect(adapterPackageIdentity(second)).toBe("deepseek-harness-acp:2.0.0:1");
    expect(adapterPackageIdentity(first)).not.toBe(adapterPackageIdentity(second));
  });

  it("fails closed when a selected catalog package disappears", () => {
    expect(
      resolveAdapterSelection(adapterManifestSelectionKey(manifest), [manifest.driver], []),
    ).toEqual({ kind: "missing" });
  });

  it("blocks required unsupported config unless a value or schema default is present", () => {
    const withoutDefault = normalizeAdapterConfigSchema({
      type: "object",
      required: ["nested"],
      properties: { nested: { type: "object" } },
    });
    expect(validateAdapterConfig(withoutDefault, undefined)).toEqual({
      nested: "Nested is required but cannot be edited by this client.",
    });
    expect(validateAdapterConfig(withoutDefault, { nested: { preserved: true } })).toEqual({});

    const withDefault = normalizeAdapterConfigSchema({
      type: "object",
      required: ["nested"],
      properties: { nested: { type: "object", default: { fromSchema: true } } },
    });
    expect(defaultAdapterConfig(withDefault)).toEqual({ nested: { fromSchema: true } });
    expect(validateAdapterConfig(withDefault, defaultAdapterConfig(withDefault))).toEqual({});
  });

  it("blocks unsupported roots without a default but accepts empty object schemas", () => {
    const unsupported = normalizeAdapterConfigSchema({ type: "array", items: { type: "string" } });
    expect(validateAdapterConfig(unsupported, defaultAdapterConfig(unsupported))).toEqual({
      $root: "This adapter's required configuration cannot be edited by this client.",
    });

    const withDefault = normalizeAdapterConfigSchema({ type: "array", default: ["agent"] });
    expect(defaultAdapterConfig(withDefault)).toEqual(["agent"]);
    expect(validateAdapterConfig(withDefault, defaultAdapterConfig(withDefault))).toEqual({});

    const emptyObject = normalizeAdapterConfigSchema({ type: "object" });
    expect(emptyObject).toMatchObject({ unsupportedRoot: false, fields: [] });
    expect(validateAdapterConfig(emptyObject, undefined)).toEqual({});
  });

  it("keeps built-in and Prime choices singular while leaving DeepSeek addable", () => {
    const prime = {
      ...manifest,
      id: ProviderAdapterPackageId.make("prime-rpc"),
      driver: ProviderDriverKind.make("primeAgent"),
    };
    expect(
      addableAdapterManifests([prime, manifest, manifest], new Set(["codex", "primeAgent"])).map(
        (entry) => entry.displayName,
      ),
    ).toEqual(["DeepSeek Harness"]);
  });

  it("resolves exact live or catalog schemas and leaves stale package identities read-only", () => {
    const instance = {
      driver: manifest.driver,
      adapterPackage: manifestPackageReference(manifest),
      enabled: true,
      config: { retries: "bad" },
    };
    expect(
      resolveAdapterConfigSchema({ instance, liveProvider: undefined, manifests: [manifest] }),
    ).toBe(schema);
    expect(
      resolveAdapterConfigSchema({ instance, liveProvider: liveProvider(), manifests: [] }),
    ).toBe(schema);

    const staleInstance = {
      ...instance,
      adapterPackage: {
        ...instance.adapterPackage,
        version: ProviderAdapterPackageVersion.make("0.9.0"),
      },
    };
    expect(
      resolveAdapterConfigSchema({
        instance: staleInstance,
        liveProvider: liveProvider(),
        manifests: [manifest],
      }),
    ).toBeUndefined();
  });
});
