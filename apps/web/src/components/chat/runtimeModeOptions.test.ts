import { ALL_PROVIDER_RUNTIME_MODES, coerceRuntimeModeToSupported } from "@t3tools/contracts";
import type { ServerProvider } from "@t3tools/contracts";
import { describe, expect, it } from "vite-plus/test";

import {
  runtimeModeOptionsForProvider,
  runtimeModeOptionsForProviders,
} from "./runtimeModeOptions";

describe("runtimeModeOptionsForProvider", () => {
  it("only offers shared modes when sending to multiple providers", () => {
    expect(
      runtimeModeOptionsForProviders([
        ALL_PROVIDER_RUNTIME_MODES,
        { supportedRuntimeModes: ["approval-required", "full-access"] },
      ]),
    ).toEqual(["approval-required", "full-access"]);
    expect(
      runtimeModeOptionsForProviders([
        { supportedRuntimeModes: ["approval-required"] },
        { supportedRuntimeModes: ["full-access"] },
      ]),
    ).toEqual([]);
  });

  it("renders Prime versus Codex modes and selects the visible provider default on switching", () => {
    const prime = {
      supportedRuntimeModes: ["approval-required", "full-access"] as const,
      defaultRuntimeMode: "approval-required" as const,
    };
    for (const provider of [prime, ALL_PROVIDER_RUNTIME_MODES]) {
      const choices = runtimeModeOptionsForProvider(provider);
      expect(choices).toEqual(provider.supportedRuntimeModes);
      const selected = coerceRuntimeModeToSupported("auto-accept-edits", provider);
      expect(choices).toContain(selected);
      expect(selected).toBe(provider === prime ? "approval-required" : "auto-accept-edits");
    }
  });

  it("keeps all current modes for a legacy provider snapshot", () => {
    expect(runtimeModeOptionsForProvider(undefined)).toEqual([
      "approval-required",
      "auto-accept-edits",
      "auto",
      "full-access",
    ]);
  });

  it("keeps only modes reported by the selected provider instance", () => {
    const provider = {
      supportedRuntimeModes: ["approval-required", "full-access"],
    } satisfies Pick<ServerProvider, "supportedRuntimeModes">;

    expect(runtimeModeOptionsForProvider(provider)).toEqual(["approval-required", "full-access"]);
  });
});
