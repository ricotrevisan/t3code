import { describe, expect, it } from "vite-plus/test";
import {
  ALL_PROVIDER_RUNTIME_MODES,
  coerceRuntimeModeToSupported,
  type ProviderRuntimeModes,
} from "./runtimeModes.ts";

const restricted: ProviderRuntimeModes = {
  supportedRuntimeModes: ["approval-required", "full-access"],
  defaultRuntimeMode: "approval-required",
};

describe("provider runtime mode selection", () => {
  it.each(["approval-required", "full-access"] as const)(
    "preserves compatible %s on provider change",
    (mode) => {
      expect(coerceRuntimeModeToSupported(mode, restricted)).toBe(mode);
    },
  );
  it.each(["auto-accept-edits", "auto"] as const)(
    "replaces incompatible %s with the advertised default",
    (mode) => {
      const selected = coerceRuntimeModeToSupported(mode, restricted);
      expect(selected).toBe("approval-required");
      expect(coerceRuntimeModeToSupported(selected, ALL_PROVIDER_RUNTIME_MODES)).toBe(
        "approval-required",
      );
    },
  );
  it.each(ALL_PROVIDER_RUNTIME_MODES.supportedRuntimeModes)(
    "preserves %s for an unrestricted provider",
    (mode) => {
      expect(coerceRuntimeModeToSupported(mode, ALL_PROVIDER_RUNTIME_MODES)).toBe(mode);
    },
  );
});
