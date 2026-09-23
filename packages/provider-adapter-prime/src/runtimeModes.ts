import type { ProviderRuntimeModes } from "@t3tools/contracts";

export const PRIME_RUNTIME_MODES: ProviderRuntimeModes = {
  supportedRuntimeModes: ["approval-required", "full-access"],
  defaultRuntimeMode: "approval-required",
};
