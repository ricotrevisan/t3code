import type { ServerProvider } from "@t3tools/contracts";
import { describe, expect, it } from "vite-plus/test";

import { runtimeModeOptionsForProvider } from "./runtimeModeOptions";

describe("runtimeModeOptionsForProvider", () => {
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
