import { describe, expect, it } from "vite-plus/test";

import {
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
