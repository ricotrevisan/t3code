import type { ModelCapabilities, ServerProviderModel } from "@t3tools/contracts";
import { createModelCapabilities } from "@t3tools/shared/model";

export const PRIME_THINKING_LEVEL_OPTION_ID = "thinkingLevel";

const PRIME_THINKING_LEVEL_ORDER = [
  "off",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
] as const;

const PRIME_THINKING_LEVEL_LABELS: Readonly<Record<string, string>> = {
  off: "Off",
  minimal: "Minimal",
  low: "Low",
  medium: "Medium",
  high: "High",
  xhigh: "Extra High",
  max: "Max",
};

const DEFAULT_THINKING_LEVEL_PREFERENCE = ["medium", "high", "low"] as const;

/**
 * GLM-5.3-Flash (Console Go / opencode-go) always thinks. Sending off/none
 * fails with: "This model always engages in thinking and cannot be disabled;
 * please use low, high, or max".
 */
const GLM_53_FLASH_THINKING_LEVEL_MAP: { readonly [x: string]: string | null } = {
  off: null,
  minimal: null,
  low: "low",
  medium: null,
  high: "high",
  xhigh: null,
  max: "max",
};

export interface PrimeListedModel {
  readonly id: string;
  readonly name?: string | undefined;
  readonly provider?: string | undefined;
  readonly thinkingLevelMap?: { readonly [x: string]: string | null } | undefined;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * Providers written by Prime `/login` into auth.json. Ambient Prime CLI or
 * environment credentials do not count as logged in.
 */
export function loggedInPrimeProvidersFromAuthData(data: unknown): ReadonlySet<string> {
  if (!isRecord(data)) {
    return new Set();
  }
  const providers = new Set<string>();
  for (const [rawProvider, credential] of Object.entries(data)) {
    const provider = rawProvider.trim();
    if (!provider || !isRecord(credential)) {
      continue;
    }
    if (credential.type === "oauth" || credential.type === "api_key") {
      providers.add(provider);
    }
  }
  return providers;
}

export function parsePrimeModelSlug(
  slug: string,
): { readonly provider: string; readonly modelId: string } | undefined {
  const trimmed = slug.trim();
  const separator = trimmed.indexOf("/");
  if (separator <= 0 || separator === trimmed.length - 1) {
    return undefined;
  }
  return {
    provider: trimmed.slice(0, separator),
    modelId: trimmed.slice(separator + 1),
  };
}

export function primeModelSlug(input: { readonly provider: string; readonly id: string }): string {
  return `${input.provider}/${input.id}`;
}

export function isPrimeGlm53FlashModelId(modelId: string): boolean {
  const id = modelId.trim().toLowerCase();
  return id === "glm-5.3-flash" || id.endsWith("/glm-5.3-flash");
}

function thinkingLevelLabel(level: string): string {
  return PRIME_THINKING_LEVEL_LABELS[level] ?? level;
}

function orderedThinkingLevels(thinkingLevelMap: {
  readonly [x: string]: string | null;
}): Array<string> {
  const available = PRIME_THINKING_LEVEL_ORDER.filter((level) => {
    const mapped = thinkingLevelMap[level];
    if (mapped === null) {
      return false;
    }
    if (level === "xhigh" || level === "max") {
      return mapped !== undefined;
    }
    return true;
  });
  const known = new Set<string>(PRIME_THINKING_LEVEL_ORDER);
  const extra = Object.keys(thinkingLevelMap).filter(
    (level) => !known.has(level) && thinkingLevelMap[level] !== null,
  );
  return [...available, ...extra];
}

export function resolvePrimeListedThinkingLevelMap(
  model: PrimeListedModel,
): { readonly [x: string]: string | null } | undefined {
  return isPrimeGlm53FlashModelId(model.id)
    ? GLM_53_FLASH_THINKING_LEVEL_MAP
    : model.thinkingLevelMap;
}

function defaultThinkingLevel(
  thinkingLevelMap: { readonly [x: string]: string | null } | undefined,
): string | undefined {
  if (thinkingLevelMap === undefined) {
    return undefined;
  }
  const levels = orderedThinkingLevels(thinkingLevelMap);
  if (levels.length === 0) {
    return undefined;
  }
  return DEFAULT_THINKING_LEVEL_PREFERENCE.find((level) => levels.includes(level)) ?? levels[0];
}

export function resolvePrimeSessionThinkingLevel(input: {
  readonly modelId: string;
  readonly requested?: string | undefined;
  readonly current?: string | undefined;
}): string | undefined {
  if (!isPrimeGlm53FlashModelId(input.modelId)) {
    return input.requested;
  }
  const levels = orderedThinkingLevels(GLM_53_FLASH_THINKING_LEVEL_MAP);
  const candidate = input.requested ?? input.current;
  if (candidate !== undefined && levels.includes(candidate)) {
    return candidate;
  }
  return defaultThinkingLevel(GLM_53_FLASH_THINKING_LEVEL_MAP);
}

export function primeThinkingCapabilities(
  thinkingLevelMap: { readonly [x: string]: string | null } | undefined,
): ModelCapabilities {
  if (thinkingLevelMap === undefined) {
    return createModelCapabilities({ optionDescriptors: [] });
  }
  const levels = orderedThinkingLevels(thinkingLevelMap);
  if (levels.length === 0) {
    return createModelCapabilities({ optionDescriptors: [] });
  }
  const defaultLevel = defaultThinkingLevel(thinkingLevelMap) ?? levels[0];
  return createModelCapabilities({
    optionDescriptors: [
      {
        id: PRIME_THINKING_LEVEL_OPTION_ID,
        label: "Thinking",
        type: "select",
        options: levels.map((level) =>
          level === defaultLevel
            ? { id: level, label: thinkingLevelLabel(level), isDefault: true as const }
            : { id: level, label: thinkingLevelLabel(level) },
        ),
        currentValue: defaultLevel,
      },
    ],
  });
}

export function mapPrimeAvailableModels(
  models: ReadonlyArray<PrimeListedModel>,
  options?: { readonly loggedInProviders?: ReadonlySet<string> },
): ReadonlyArray<ServerProviderModel> {
  const mapped: Array<ServerProviderModel> = [];
  const loggedInProviders = options?.loggedInProviders;
  for (const model of models) {
    const id = model.id.trim();
    const provider = model.provider?.trim();
    if (!id || !provider) {
      continue;
    }
    if (loggedInProviders !== undefined && !loggedInProviders.has(provider)) {
      continue;
    }
    const name = model.name?.trim() || id;
    mapped.push({
      slug: primeModelSlug({ provider, id }),
      name,
      subProvider: provider,
      isCustom: false,
      capabilities: primeThinkingCapabilities(resolvePrimeListedThinkingLevelMap(model)),
    });
  }
  return mapped.toSorted((left, right) => left.name.localeCompare(right.name));
}
