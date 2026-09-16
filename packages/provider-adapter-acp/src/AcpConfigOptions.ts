import type {
  ProviderOptionSelection,
  SelectProviderOptionDescriptor,
  ServerProviderModel,
} from "@t3tools/contracts";
import type * as AcpSchema from "effect-acp/schema";

export const ACP_MODEL_CONFIG_ID = "model";
export const ACP_REASONING_CONFIG_ID = "reasoning_effort";
export const T3_REASONING_OPTION_ID = "reasoningEffort";
export const ACP_EMPTY_VALUE_SENTINEL = "__t3_acp_empty__";

type AcpSelectConfigOption = Extract<AcpSchema.SessionConfigOption, { readonly type: "select" }>;

type AcpSelectOptions = AcpSelectConfigOption["options"];

export interface FlattenedAcpSelectOption {
  readonly value: string;
  readonly name: string;
  readonly description?: string;
  readonly groupId?: string;
  readonly groupName?: string;
}

export interface ProjectedAcpSelectConfig {
  readonly configId: string;
  /** Canonical values exposed to T3. Unsafe wire values use non-empty sentinels. */
  readonly values: ReadonlyArray<string>;
  readonly currentValue?: string;
  readonly canonicalToWireValue: ReadonlyMap<string, string>;
  readonly wireToCanonicalValue: ReadonlyMap<string, string>;
}

export interface ProjectedAcpReasoningConfig extends ProjectedAcpSelectConfig {
  readonly descriptor: SelectProviderOptionDescriptor;
}

const nonEmpty = (value: string | null | undefined): string | undefined => {
  const trimmed = value?.trim();
  return trimmed ? trimmed : undefined;
};

/** Flatten either standard ACP select shape without changing its opaque values. */
export function flattenAcpSelectOptions(
  input: AcpSelectOptions | AcpSelectConfigOption | null | undefined,
): ReadonlyArray<FlattenedAcpSelectOption> {
  if (!input) return [];
  const options = (
    Array.isArray(input) ? input : (input as AcpSelectConfigOption).options
  ) as ReadonlyArray<AcpSchema.SessionConfigSelectOption | AcpSchema.SessionConfigSelectGroup>;
  const flattened: Array<FlattenedAcpSelectOption> = [];

  for (const entry of options) {
    if ("value" in entry) {
      const description = nonEmpty(entry.description);
      flattened.push({
        value: entry.value,
        name: nonEmpty(entry.name) ?? (entry.value.length > 0 ? entry.value : "Agent default"),
        ...(description ? { description } : {}),
      });
      continue;
    }

    const groupName = nonEmpty(entry.name) ?? nonEmpty(entry.group);
    for (const option of entry.options) {
      const description = nonEmpty(option.description);
      flattened.push({
        value: option.value,
        name: nonEmpty(option.name) ?? (option.value.length > 0 ? option.value : "Agent default"),
        ...(description ? { description } : {}),
        ...(entry.group ? { groupId: entry.group } : {}),
        ...(groupName ? { groupName } : {}),
      });
    }
  }

  return flattened;
}

/** ACP configuration ids are opaque and therefore matched exactly. */
export function findAcpConfigOption(
  options: ReadonlyArray<AcpSchema.SessionConfigOption> | null | undefined,
  configId: string,
): AcpSchema.SessionConfigOption | undefined {
  return options?.find((option) => option.id === configId);
}

const findModelConfigOption = (
  options: ReadonlyArray<AcpSchema.SessionConfigOption>,
): AcpSelectConfigOption | undefined => {
  const standard = options.find(
    (option): option is AcpSelectConfigOption =>
      option.type === "select" && option.category === "model",
  );
  if (standard) return standard;

  const compatibility = findAcpConfigOption(options, ACP_MODEL_CONFIG_ID);
  return compatibility?.type === "select" ? compatibility : undefined;
};

const findReasoningConfigOption = (
  options: ReadonlyArray<AcpSchema.SessionConfigOption>,
): AcpSelectConfigOption | undefined => {
  const standard = options.find(
    (option): option is AcpSelectConfigOption =>
      option.type === "select" && option.category === "thought_level",
  );
  if (standard) return standard;

  const compatibility = findAcpConfigOption(options, ACP_REASONING_CONFIG_ID);
  return compatibility?.type === "select" ? compatibility : undefined;
};

const uniqueOptions = (
  options: ReadonlyArray<FlattenedAcpSelectOption>,
): ReadonlyArray<FlattenedAcpSelectOption> => {
  const seen = new Set<string>();
  return options.filter((option) => {
    if (seen.has(option.value)) return false;
    seen.add(option.value);
    return true;
  });
};

interface CanonicalAcpSelectOptions {
  readonly choices: ReadonlyArray<FlattenedAcpSelectOption & { readonly canonicalValue: string }>;
  readonly values: ReadonlyArray<string>;
  readonly canonicalToWireValue: ReadonlyMap<string, string>;
  readonly wireToCanonicalValue: ReadonlyMap<string, string>;
}

const canonicalizeOptions = (
  choices: ReadonlyArray<FlattenedAcpSelectOption>,
): CanonicalAcpSelectOptions => {
  const wireValues = new Set(choices.map((choice) => choice.value));
  const usedCanonicalValues = new Set<string>();
  let sentinelIndex = 0;
  const nextSentinel = () => {
    let candidate = ACP_EMPTY_VALUE_SENTINEL;
    while (wireValues.has(candidate) || usedCanonicalValues.has(candidate)) {
      sentinelIndex += 1;
      candidate = `${ACP_EMPTY_VALUE_SENTINEL}:${sentinelIndex}`;
    }
    usedCanonicalValues.add(candidate);
    return candidate;
  };

  const canonicalToWireValue = new Map<string, string>();
  const wireToCanonicalValue = new Map<string, string>();
  const canonicalChoices = choices.map((choice) => {
    const isT3Safe = choice.value.length > 0 && choice.value.trim() === choice.value;
    const canonicalValue = isT3Safe ? choice.value : nextSentinel();
    usedCanonicalValues.add(canonicalValue);
    canonicalToWireValue.set(canonicalValue, choice.value);
    wireToCanonicalValue.set(choice.value, canonicalValue);
    return { ...choice, canonicalValue };
  });

  return {
    choices: canonicalChoices,
    values: canonicalChoices.map((choice) => choice.canonicalValue),
    canonicalToWireValue,
    wireToCanonicalValue,
  };
};

const currentValueIn = (
  option: AcpSelectConfigOption,
  wireToCanonicalValue: ReadonlyMap<string, string>,
): string | undefined => wireToCanonicalValue.get(option.currentValue);

const projectReasoningConfig = (
  option: AcpSelectConfigOption | undefined,
): ProjectedAcpReasoningConfig | undefined => {
  if (!option) return undefined;
  const choices = uniqueOptions(flattenAcpSelectOptions(option));
  if (choices.length === 0) return undefined;

  const canonical = canonicalizeOptions(choices);
  const currentValue = currentValueIn(option, canonical.wireToCanonicalValue);
  const descriptor = {
    id: T3_REASONING_OPTION_ID,
    label: nonEmpty(option.name) ?? "Reasoning",
    type: "select",
    options: canonical.choices.map((choice) => ({
      id: choice.canonicalValue,
      label: choice.name,
      ...(choice.description ? { description: choice.description } : {}),
      ...(choice.canonicalValue === currentValue ? { isDefault: true } : {}),
    })),
    ...(nonEmpty(option.description) ? { description: nonEmpty(option.description) } : {}),
    ...(currentValue ? { currentValue } : {}),
  } satisfies SelectProviderOptionDescriptor;

  return {
    configId: option.id,
    values: canonical.values,
    canonicalToWireValue: canonical.canonicalToWireValue,
    wireToCanonicalValue: canonical.wireToCanonicalValue,
    descriptor,
    ...(currentValue ? { currentValue } : {}),
  };
};

/**
 * Project standard ACP model and reasoning selectors into T3 snapshot data.
 * The separate configs let a runtime apply the model first, then re-project
 * refreshed options before applying model-dependent reasoning.
 */
export function projectAcpConfigOptions(
  options: ReadonlyArray<AcpSchema.SessionConfigOption> | null | undefined,
) {
  const configOptions = options ?? [];
  const selectModelOption = findModelConfigOption(configOptions);
  const modelChoices = selectModelOption
    ? uniqueOptions(flattenAcpSelectOptions(selectModelOption))
    : [];
  const canonicalModels = canonicalizeOptions(modelChoices);
  const currentModel = selectModelOption
    ? currentValueIn(selectModelOption, canonicalModels.wireToCanonicalValue)
    : undefined;
  const reasoningConfig = projectReasoningConfig(findReasoningConfigOption(configOptions));
  const capabilities = reasoningConfig ? { optionDescriptors: [reasoningConfig.descriptor] } : null;

  const models = canonicalModels.choices.map(
    (choice) =>
      ({
        slug: choice.canonicalValue,
        name: choice.name,
        ...(choice.groupName ? { subProvider: choice.groupName } : {}),
        isCustom: false,
        ...(choice.canonicalValue === currentModel ? { isDefault: true } : {}),
        capabilities,
      }) satisfies ServerProviderModel,
  );

  const modelConfig: ProjectedAcpSelectConfig | undefined = selectModelOption
    ? {
        configId: selectModelOption.id,
        values: canonicalModels.values,
        canonicalToWireValue: canonicalModels.canonicalToWireValue,
        wireToCanonicalValue: canonicalModels.wireToCanonicalValue,
        ...(currentModel ? { currentValue: currentModel } : {}),
      }
    : undefined;

  return { models, modelConfig, reasoningConfig };
}

export type AcpConfigOptionsProjection = ReturnType<typeof projectAcpConfigOptions>;

/** Find T3's selected reasoning value without normalizing the provider token. */
export function findAcpReasoningSelection(
  selections: ReadonlyArray<ProviderOptionSelection> | null | undefined,
): string | undefined {
  const value = selections?.find((selection) => selection.id === T3_REASONING_OPTION_ID)?.value;
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

/** Build one validated update. Call this for model before refreshed reasoning. */
export function getAcpConfigOptionUpdate(
  config: ProjectedAcpSelectConfig | undefined,
  requestedValue: string | null | undefined,
): { readonly configId: string; readonly value: string } | undefined {
  if (
    !config ||
    requestedValue === undefined ||
    requestedValue === null ||
    requestedValue.length === 0 ||
    requestedValue === config.currentValue ||
    !config.values.includes(requestedValue)
  ) {
    return undefined;
  }

  const wireValue = config.canonicalToWireValue.get(requestedValue);
  if (wireValue === undefined) return undefined;
  return { configId: config.configId, value: wireValue };
}
