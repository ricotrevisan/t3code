import { describe, expect, it } from "vite-plus/test";
import * as Schema from "effect/Schema";

import {
  isProviderAdapterManifestCompatible,
  ProviderAdapterManifestCatalog,
  ProviderAdapterManifestV1,
  ProviderAdapterProtocolCapabilitiesV1,
  T3_PROVIDER_ADAPTER_PROTOCOL_VERSION,
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

  it("rejects packages whose host protocol range excludes this host", () => {
    const decoded = decodeManifest(manifest({ hostProtocol: { minimum: 2, maximum: 3 } }));
    expect(isProviderAdapterManifestCompatible(decoded)).toBe(false);
  });

  it("represents Pi RPC constraints without a Pi-specific core field", () => {
    const pi = decodeManifest(
      manifest({
        id: "pi-rpc",
        driver: "piRpc",
        displayName: "Pi RPC",
        capabilities: [
          "session.resume",
          "turn.steer",
          "turn.follow-up",
          "turn.interrupt",
          "input.attachments",
          "request.structured-input",
          "model.discovery",
          "model.switch",
          "reasoning.selection",
          "stream.reasoning",
          "stream.tool-lifecycle",
          "stream.usage",
          "stream.context",
        ],
      }),
    );
    expect(pi.transport.sessionConcurrency).toBe("one-per-process");
    expect(pi.capabilities).not.toContain("request.approval");
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
