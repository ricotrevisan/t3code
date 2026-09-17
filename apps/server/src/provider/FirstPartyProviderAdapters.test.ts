import { describe, expect, it } from "@effect/vitest";
import {
  PRIME_PROVIDER_ADAPTER_DEFAULT_CONFIG,
  PRIME_PROVIDER_ADAPTER_PACKAGE,
  PRIME_PROVIDER_ADAPTER_PACKAGE_REFERENCE as PACKAGE_PRIME_PROVIDER_ADAPTER_PACKAGE_REFERENCE,
  PrimeProviderAdapterConfig,
} from "@t3tools/provider-adapter-prime";
import * as Schema from "effect/Schema";

import {
  DEEPSEEK_HARNESS_ADAPTER_DEFAULT_CONFIG,
  DEEPSEEK_HARNESS_ADAPTER_MANIFEST,
  DEEPSEEK_HARNESS_ADAPTER_PACKAGE,
  DEEPSEEK_HARNESS_ADAPTER_PACKAGE_REFERENCE,
  DEEPSEEK_HARNESS_DRIVER_KIND,
  DeepSeekHarnessAdapterConfig,
} from "./DeepSeekHarnessAdapter.ts";
import {
  FIRST_PARTY_PROVIDER_ADAPTER_DRIVERS,
  PRIME_PROVIDER_ADAPTER_MANIFEST,
  PRIME_PROVIDER_ADAPTER_PACKAGE_REFERENCE,
} from "./FirstPartyProviderAdapters.ts";

const decodeDeepSeekHarnessAdapterConfig = Schema.decodeUnknownSync(DeepSeekHarnessAdapterConfig);

describe("DeepSeek Harness first-party ACP package", () => {
  it("declares the DeepSeek Harness identity, transport, and exact capability ceiling", () => {
    expect(DEEPSEEK_HARNESS_ADAPTER_MANIFEST).toMatchObject({
      protocolVersion: 1,
      id: "deepseek-harness-acp",
      version: "1.0.0",
      driver: "deepseekHarness",
      displayName: "DeepSeek Harness",
      hostProtocol: { minimum: 1, maximum: 1 },
      transport: {
        kind: "supervised-stdio",
        protocol: "acp-v1",
        sessionConcurrency: "multiplexed",
      },
    });
    expect(DEEPSEEK_HARNESS_ADAPTER_MANIFEST.capabilities).toEqual([
      "session.resume",
      "turn.interrupt",
      "request.approval",
      "model.discovery",
      "model.switch",
      "reasoning.selection",
      "stream.reasoning",
      "stream.tool-lifecycle",
      "stream.context",
    ]);
  });

  it("exposes only process launch config and defaults to dsh --profile acp", () => {
    expect(DEEPSEEK_HARNESS_ADAPTER_MANIFEST.configSchema).toEqual({
      type: "object",
      additionalProperties: false,
      required: ["command", "args"],
      properties: {
        command: {
          type: "string",
          default: "dsh",
          description: "Path to the DeepSeek Harness executable.",
        },
        args: {
          type: "array",
          items: { type: "string" },
          default: ["--profile", "acp"],
          description: "Arguments passed to DeepSeek Harness.",
        },
        cwd: {
          type: "string",
          description: "Optional working directory for the DeepSeek Harness process.",
        },
      },
    });
    expect(DEEPSEEK_HARNESS_ADAPTER_DEFAULT_CONFIG).toEqual({
      command: "dsh",
      args: ["--profile", "acp"],
    });
    expect(DEEPSEEK_HARNESS_ADAPTER_PACKAGE.defaultConfig()).toEqual(
      DEEPSEEK_HARNESS_ADAPTER_DEFAULT_CONFIG,
    );
    expect(DEEPSEEK_HARNESS_ADAPTER_MANIFEST.configSchema.properties).not.toHaveProperty("env");
    expect(DEEPSEEK_HARNESS_ADAPTER_PACKAGE.configSchema).toBe(DeepSeekHarnessAdapterConfig);
    expect(
      decodeDeepSeekHarnessAdapterConfig({
        command: "dsh",
        args: ["--profile", "acp"],
        env: { DEEPSEEK_API_KEY: "secret" },
      }),
    ).toEqual({ command: "dsh", args: ["--profile", "acp"] });
  });

  it("registers the package and driver identity without creating an instance", () => {
    expect(DEEPSEEK_HARNESS_ADAPTER_PACKAGE.manifest).toBe(DEEPSEEK_HARNESS_ADAPTER_MANIFEST);
    const driver = FIRST_PARTY_PROVIDER_ADAPTER_DRIVERS.find(
      (candidate) => candidate.driverKind === DEEPSEEK_HARNESS_DRIVER_KIND,
    );
    expect(driver).toBeDefined();
    expect(driver?.adapterPackage).toEqual(DEEPSEEK_HARNESS_ADAPTER_PACKAGE_REFERENCE);
    expect(driver?.defaultConfig()).toEqual(DEEPSEEK_HARNESS_ADAPTER_DEFAULT_CONFIG);
  });
});

describe("Prime first-party provider adapter package", () => {
  it("re-exports the package identity and exact capability ceiling", () => {
    expect(PRIME_PROVIDER_ADAPTER_MANIFEST).toBe(PRIME_PROVIDER_ADAPTER_PACKAGE.manifest);
    expect(PRIME_PROVIDER_ADAPTER_PACKAGE_REFERENCE).toBe(
      PACKAGE_PRIME_PROVIDER_ADAPTER_PACKAGE_REFERENCE,
    );
    expect(PRIME_PROVIDER_ADAPTER_MANIFEST).toMatchObject({
      protocolVersion: 1,
      id: "prime-rpc",
      version: "1.0.0",
      driver: "primeAgent",
      displayName: "Prime Agent",
      hostProtocol: { minimum: 2, maximum: 2 },
    });
    expect(PRIME_PROVIDER_ADAPTER_MANIFEST.capabilities).toEqual([
      "session.resume",
      "turn.steer",
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
      "stream.subagents",
    ]);
  });

  it("registers Prime through the external driver with the package config", () => {
    const driver = FIRST_PARTY_PROVIDER_ADAPTER_DRIVERS.find(
      (candidate) => candidate.driverKind === PRIME_PROVIDER_ADAPTER_MANIFEST.driver,
    );

    expect(FIRST_PARTY_PROVIDER_ADAPTER_DRIVERS).toHaveLength(2);
    expect(driver).toBeDefined();
    expect(driver?.adapterPackage).toEqual(PRIME_PROVIDER_ADAPTER_PACKAGE_REFERENCE);
    expect(driver?.configSchema).toBe(PrimeProviderAdapterConfig);
    expect(driver?.defaultConfig()).toEqual(PRIME_PROVIDER_ADAPTER_DEFAULT_CONFIG);
    expect(PRIME_PROVIDER_ADAPTER_MANIFEST.configSchema.properties).not.toHaveProperty("enabled");
  });
});
