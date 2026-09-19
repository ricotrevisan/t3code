import { describe, expect, it } from "vite-plus/test";

import {
  mapPiModelIdentity,
  PI_PROVIDER_ADAPTER_CAPABILITIES,
  PI_PROVIDER_ADAPTER_DEFAULT_CONFIG,
  PI_PROVIDER_ADAPTER_MANIFEST,
  PI_PROVIDER_ADAPTER_PACKAGE,
  PI_PROVIDER_ADAPTER_PACKAGE_REFERENCE,
} from "./index.ts";

describe("Pi provider adapter package", () => {
  it("exports the shipped identity and real Pi defaults", () => {
    expect(PI_PROVIDER_ADAPTER_MANIFEST).toMatchObject({
      id: "pi-rpc",
      version: "1.0.0",
      driver: "piRpc",
      displayName: "Pi",
      hostProtocol: { minimum: 2, maximum: 2 },
    });
    expect(PI_PROVIDER_ADAPTER_DEFAULT_CONFIG).toEqual({
      binaryPath: "pi",
      args: [],
    });
    expect(PI_PROVIDER_ADAPTER_PACKAGE_REFERENCE).toEqual({
      id: "pi-rpc",
      version: "1.0.0",
      protocolVersion: 1,
    });
    expect(PI_PROVIDER_ADAPTER_PACKAGE.manifest).toBe(PI_PROVIDER_ADAPTER_MANIFEST);
  });

  it("advertises only implemented capabilities", () => {
    expect(PI_PROVIDER_ADAPTER_CAPABILITIES.features).toContain("reasoning.selection");
    expect(PI_PROVIDER_ADAPTER_CAPABILITIES.features).not.toContain("request.approval");
    expect(PI_PROVIDER_ADAPTER_CAPABILITIES.features).not.toContain("conversation.rollback");
  });
});

describe("mapPiModelIdentity", () => {
  it("labels the account, so two same-named models from different accounts stay apart", () => {
    const primary = mapPiModelIdentity({
      provider: "openai-codex",
      id: "gpt-5.6-sol",
      name: "GPT-5.6 Sol",
    });
    const thirdAccount = mapPiModelIdentity({
      provider: "openai-codex-3",
      id: "gpt-5.6-sol",
      name: "GPT-5.6 Sol",
    });

    expect(primary).toEqual({
      slug: "openai-codex/gpt-5.6-sol",
      name: "GPT-5.6 Sol",
      subProvider: "openai-codex",
    });
    expect(thirdAccount.name).toBe(primary.name);
    expect(thirdAccount.slug).not.toBe(primary.slug);
    expect(thirdAccount.subProvider).not.toBe(primary.subProvider);
  });

  it("keeps the provider in the slug and the account label", () => {
    expect(
      mapPiModelIdentity({
        provider: "openrouter",
        id: "openai/gpt-5.6-sol",
        name: "OpenAI: GPT-5.6 Sol",
      }),
    ).toEqual({
      slug: "openrouter/openai/gpt-5.6-sol",
      name: "OpenAI: GPT-5.6 Sol",
      subProvider: "openrouter",
    });
  });

  it("falls back to the model id and omits a blank account label", () => {
    const identity = mapPiModelIdentity({ provider: "   ", id: "gpt-5.6-sol" });

    expect(identity.name).toBe("gpt-5.6-sol");
    expect(identity).not.toHaveProperty("subProvider");
  });
});
