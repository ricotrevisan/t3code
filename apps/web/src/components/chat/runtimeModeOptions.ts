import type { RuntimeMode, ServerProvider } from "@t3tools/contracts";

export const ALL_RUNTIME_MODE_OPTIONS: ReadonlyArray<RuntimeMode> = [
  "approval-required",
  "auto-accept-edits",
  "auto",
  "full-access",
];

export function runtimeModeOptionsForProvider(
  provider: Pick<ServerProvider, "supportedRuntimeModes"> | null | undefined,
): ReadonlyArray<RuntimeMode> {
  if (provider?.supportedRuntimeModes === undefined) {
    return ALL_RUNTIME_MODE_OPTIONS;
  }
  const supported = new Set(provider.supportedRuntimeModes);
  return ALL_RUNTIME_MODE_OPTIONS.filter((mode) => supported.has(mode));
}
