import * as Schema from "effect/Schema";

export const RuntimeMode = Schema.Literals([
  "approval-required",
  "auto-accept-edits",
  "auto",
  "full-access",
]);
export type RuntimeMode = typeof RuntimeMode.Type;
export const DEFAULT_RUNTIME_MODE: RuntimeMode = "full-access";

/** Canonical execution policies advertised by a provider, independent of models. */
export const ProviderRuntimeModes = Schema.Struct({
  supportedRuntimeModes: Schema.Array(RuntimeMode).check(Schema.isMinLength(1), Schema.isUnique()),
  defaultRuntimeMode: RuntimeMode,
}).check(
  Schema.makeFilter(
    (value) =>
      value.supportedRuntimeModes.includes(value.defaultRuntimeMode) ||
      "The default runtime mode must be supported by the provider.",
  ),
);
export type ProviderRuntimeModes = typeof ProviderRuntimeModes.Type;

export const ALL_PROVIDER_RUNTIME_MODES: ProviderRuntimeModes = {
  supportedRuntimeModes: ["approval-required", "auto-accept-edits", "auto", "full-access"],
  defaultRuntimeMode: DEFAULT_RUNTIME_MODE,
};

/** Client selection reconciliation; servers must reject unsupported requests instead. */
export function coerceRuntimeModeToSupported(
  currentMode: RuntimeMode,
  provider:
    | {
        readonly supportedRuntimeModes?: ReadonlyArray<RuntimeMode> | undefined;
        readonly defaultRuntimeMode?: RuntimeMode | undefined;
      }
    | null
    | undefined,
): RuntimeMode {
  const supported = provider?.supportedRuntimeModes;
  if (!supported?.length || supported.includes(currentMode)) return currentMode;
  if (provider?.defaultRuntimeMode && supported.includes(provider.defaultRuntimeMode)) {
    return provider.defaultRuntimeMode;
  }
  // Older servers advertised only a list; retain their selection behavior.
  return supported.includes("full-access") ? "full-access" : supported[0]!;
}
