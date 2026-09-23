import { ALL_PROVIDER_RUNTIME_MODES, coerceRuntimeModeToSupported } from "@t3tools/contracts";
import type { ProviderOptionDescriptor } from "@t3tools/contracts";
import { describe, expect, it } from "vite-plus/test";

import { runtimeModeChoices, selectableChoices } from "./thread-settings-options";

const effortDescriptor: Extract<ProviderOptionDescriptor, { type: "select" }> = {
  id: "effort",
  label: "Reasoning",
  type: "select",
  options: [
    { id: "low", label: "Low" },
    { id: "medium", label: "Medium", isDefault: true },
    { id: "high", label: "High" },
    { id: "ultrathink", label: "Ultrathink" },
    { id: "ultracode", label: "Ultracode" },
  ],
  currentValue: "high",
  promptInjectedValues: ["ultrathink"],
};

describe("runtimeModeChoices", () => {
  it("renders Prime versus Codex modes and selects the visible provider default on switching", () => {
    const prime = {
      supportedRuntimeModes: ["approval-required", "full-access"] as const,
      defaultRuntimeMode: "approval-required" as const,
    };
    for (const provider of [prime, ALL_PROVIDER_RUNTIME_MODES]) {
      const choices = runtimeModeChoices(provider.supportedRuntimeModes).map(
        (choice) => choice.mode,
      );
      expect(choices).toEqual(provider.supportedRuntimeModes);
      const selected = coerceRuntimeModeToSupported("auto-accept-edits", provider);
      expect(choices).toContain(selected);
      expect(selected).toBe(provider === prime ? "approval-required" : "auto-accept-edits");
    }
  });

  it("keeps all current modes for legacy provider snapshots", () => {
    expect(runtimeModeChoices(undefined).map((choice) => choice.mode)).toEqual([
      "approval-required",
      "auto-accept-edits",
      "auto",
      "full-access",
    ]);
  });

  it("keeps only modes reported by the selected provider instance", () => {
    expect(
      runtimeModeChoices(["approval-required", "full-access"]).map((choice) => choice.mode),
    ).toEqual(["approval-required", "full-access"]);
  });
});

describe("selectableChoices", () => {
  it("hides prompt-injected and workflow-trigger choices, keeping declared order", () => {
    expect(selectableChoices(effortDescriptor).map((choice) => choice.id)).toEqual([
      "low",
      "medium",
      "high",
    ]);
  });
});
