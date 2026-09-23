import { describe, expect, it } from "vite-plus/test";
import * as Schema from "effect/Schema";

import {
  isProviderAdapterManifestCompatible,
  ProviderAdapterManifestCatalog,
  ProviderAdapterManifestV1,
  ProviderAdapterProtocolCapabilitiesV1,
  T3_PROVIDER_ADAPTER_HOST_PROTOCOL_VERSION,
  T3_PROVIDER_ADAPTER_PROTOCOL_VERSION,
  T3_PROVIDER_ADAPTER_SUPPORTED_HOST_PROTOCOL_VERSIONS,
} from "./providerAdapter.ts";

const decodeManifest = Schema.decodeUnknownSync(ProviderAdapterManifestV1);
const decodeManifestCatalog = Schema.decodeUnknownSync(ProviderAdapterManifestCatalog);
const encodeManifestCatalog = Schema.encodeSync(ProviderAdapterManifestCatalog);
const decodeCapabilities = Schema.decodeUnknownSync(ProviderAdapterProtocolCapabilitiesV1);

const manifest = (overrides: Record<string, unknown> = {}) => ({
  protocolVersion: T3_PROVIDER_ADAPTER_PROTOCOL_VERSION,
  id: "fixture-adapter",
  version: "1.2.3",
  driver: "fixtureHarness",
  displayName: "Fixture Harness",
  hostProtocol: { minimum: 1, maximum: 1 },
  transport: {
    kind: "supervised-stdio",
    protocol: "jsonl-rpc",
    sessionConcurrency: "one-per-process",
  },
  capabilities: ["session.resume", "turn.interrupt", "stream.tool-lifecycle"],
  configSchema: { type: "object", properties: {} },
  ...overrides,
});

describe("ProviderAdapterManifestV1", () => {
  it("decodes a versioned trusted-local stdio adapter manifest", () => {
    const decoded = decodeManifest(manifest());
    expect(decoded.id).toBe("fixture-adapter");
    expect(decoded.transport).toEqual({
      kind: "supervised-stdio",
      protocol: "jsonl-rpc",
      sessionConcurrency: "one-per-process",
    });
    expect(isProviderAdapterManifestCompatible(decoded)).toBe(true);
  });

  it("decodes optional package-managed harness maintenance metadata", () => {
    expect(decodeManifest(manifest()).maintenance).toBeUndefined();
    expect(
      decodeManifest(
        manifest({
          maintenance: {
            npmPackage: "@example/fixture-harness",
            binaryConfigKey: "binaryPath",
          },
        }),
      ).maintenance,
    ).toEqual({
      npmPackage: "@example/fixture-harness",
      binaryConfigKey: "binaryPath",
    });
    expect(() =>
      decodeManifest(manifest({ maintenance: { npmPackage: "", binaryConfigKey: "binaryPath" } })),
    ).toThrow();
  });

  it("fails closed for a future adapter protocol version", () => {
    expect(() => decodeManifest(manifest({ protocolVersion: 2 }))).toThrow();
  });

  it("rejects duplicate or unknown capability declarations", () => {
    expect(() =>
      decodeManifest(manifest({ capabilities: ["session.resume", "session.resume"] })),
    ).toThrow();
    expect(() => decodeManifest(manifest({ capabilities: ["session.teleport"] }))).toThrow();
  });

  it("rejects executable values in the client-visible configuration schema", () => {
    expect(() =>
      decodeManifest(
        manifest({ configSchema: { type: "object", transform: () => "not declarative" } }),
      ),
    ).toThrow();
  });

  it("supports both legacy and current host protocol versions", () => {
    expect(T3_PROVIDER_ADAPTER_HOST_PROTOCOL_VERSION).toBe(2);
    expect(T3_PROVIDER_ADAPTER_SUPPORTED_HOST_PROTOCOL_VERSIONS).toEqual([1, 2]);

    const legacy = decodeManifest(manifest({ hostProtocol: { minimum: 1, maximum: 1 } }));
    const current = decodeManifest(manifest({ hostProtocol: { minimum: 2, maximum: 2 } }));
    expect(isProviderAdapterManifestCompatible(legacy)).toBe(true);
    expect(isProviderAdapterManifestCompatible(current)).toBe(true);
  });

  it("accepts an explicit host version or supported-version list", () => {
    const legacy = decodeManifest(manifest({ hostProtocol: { minimum: 1, maximum: 1 } }));
    expect(isProviderAdapterManifestCompatible(legacy, 1)).toBe(true);
    expect(isProviderAdapterManifestCompatible(legacy, 2)).toBe(false);
    expect(isProviderAdapterManifestCompatible(legacy, [2, 1])).toBe(true);
  });

  it("rejects invalid or non-overlapping host protocol ranges", () => {
    const future = decodeManifest(manifest({ hostProtocol: { minimum: 3, maximum: 4 } }));
    const inverted = decodeManifest(manifest({ hostProtocol: { minimum: 2, maximum: 1 } }));
    expect(isProviderAdapterManifestCompatible(future)).toBe(false);
    expect(isProviderAdapterManifestCompatible(inverted)).toBe(false);
  });

  it("represents Pi RPC constraints without a Pi-specific core field", () => {
    const pi = decodeManifest(
      manifest({
        id: "pi-rpc",
        driver: "piRpc",
        displayName: "Pi",
        hostProtocol: { minimum: 2, maximum: 2 },
        capabilities: [
          "session.resume",
          "input.attachments",
          "turn.steer",
          "turn.interrupt",
          "request.structured-input",
          "model.discovery",
          "model.switch",
          "reasoning.selection",
          "stream.reasoning",
          "stream.tool-lifecycle",
          "stream.usage",
        ],
      }),
    );
    expect(pi.hostProtocol).toEqual({ minimum: 2, maximum: 2 });
    expect(pi.transport.sessionConcurrency).toBe("one-per-process");
    expect(pi.capabilities).not.toContain("request.approval");
    expect(pi.capabilities).toContain("input.attachments");
    expect(pi.capabilities).not.toContain("stream.context");
    expect(pi.capabilities).not.toContain("stream.subagents");
  });

  it("represents ACP/DeepSeek capabilities without a DeepSeek-specific field", () => {
    const acp = decodeManifest(
      manifest({
        id: "acp-stdio",
        driver: "acpStdio",
        displayName: "ACP stdio",
        transport: {
          kind: "supervised-stdio",
          protocol: "acp-v1",
          sessionConcurrency: "multiplexed",
        },
        capabilities: [
          "session.resume",
          "turn.interrupt",
          "input.attachments",
          "request.approval",
          "model.discovery",
          "model.switch",
          "reasoning.selection",
          "stream.reasoning",
          "stream.tool-lifecycle",
          "stream.context",
        ],
      }),
    );
    expect(acp.transport.protocol).toBe("acp-v1");
    expect(acp.capabilities).not.toContain("turn.steer");
    expect(acp.capabilities).not.toContain("request.structured-input");
  });
});

describe("ProviderAdapterManifestCatalog", () => {
  it("round-trips only validated declarative manifests", () => {
    const decoded = decodeManifestCatalog([
      manifest({
        modulePath: "/trusted-local/fixture.mjs",
        registration: { enabled: true },
      }),
    ]);
    const encoded = encodeManifestCatalog(decoded);

    expect(encoded).toHaveLength(1);
    expect(encoded[0]).not.toHaveProperty("modulePath");
    expect(encoded[0]).not.toHaveProperty("registration");
    expect(decodeManifestCatalog(encoded)).toEqual(decoded);
  });

  it("drops manifest versions this client does not understand", () => {
    const decoded = decodeManifestCatalog([manifest(), manifest({ protocolVersion: 2 })]);

    expect(decoded).toEqual([decodeManifest(manifest())]);
  });
});

describe("ProviderAdapterProtocolCapabilitiesV1", () => {
  it("decodes negotiated instance capabilities independently from package potential", () => {
    expect(
      decodeCapabilities({
        protocolVersion: 1,
        features: ["turn.interrupt", "stream.reasoning"],
      }),
    ).toEqual({ protocolVersion: 1, features: ["turn.interrupt", "stream.reasoning"] });
  });
});
