import type {
  ProviderOptionSelection,
  SelectProviderOptionDescriptor,
  ServerProviderModel,
} from "@t3tools/contracts";
import type * as AcpSchema from "effect-acp/schema";

export const ACP_MODEL_CONFIG_ID = "model";
export const ACP_REASONING_CONFIG_ID = "reasoning_effort";
export const T3_REASONING_OPTION_ID = "reasoningEffort";

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
  readonly currentValue?: string;
  readonly values: ReadonlyArray<string>;
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
      if (entry.value.trim().length === 0) continue;
      const description = nonEmpty(entry.description);
      flattened.push({
        value: entry.value,
        name: nonEmpty(entry.name) ?? entry.value,
        ...(description ? { description } : {}),
      });
      continue;
    }

    const groupName = nonEmpty(entry.name) ?? nonEmpty(entry.group);
    for (const option of entry.options) {
      if (option.value.trim().length === 0) continue;
      const description = nonEmpty(option.description);
      flattened.push({
        value: option.value,
        name: nonEmpty(option.name) ?? option.value,
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

const findReasoningConfigOption = (
  options: ReadonlyArray<AcpSchema.SessionConfigOption>,
): AcpSelectConfigOption | undefined => {
  const exact = findAcpConfigOption(options, ACP_REASONING_CONFIG_ID);
  if (exact?.type === "select") return exact;
  return options.find(
    (option): option is AcpSelectConfigOption =>
      option.type === "select" && option.category === "thought_level",
  );
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

const currentValueIn = (
  option: AcpSelectConfigOption,
  values: ReadonlyArray<string>,
): string | undefined =>
  option.currentValue.length > 0 && values.includes(option.currentValue)
    ? option.currentValue
    : undefined;

const projectReasoningConfig = (
  option: AcpSelectConfigOption | undefined,
): ProjectedAcpReasoningConfig | undefined => {
  if (!option) return undefined;
  const choices = uniqueOptions(flattenAcpSelectOptions(option));
  if (choices.length === 0) return undefined;

  const values = choices.map((choice) => choice.value);
  const currentValue = currentValueIn(option, values);
  const descriptor = {
    id: T3_REASONING_OPTION_ID,
    label: nonEmpty(option.name) ?? "Reasoning",
    type: "select",
    options: choices.map((choice) => ({
      id: choice.value,
      label: choice.name,
      ...(choice.description ? { description: choice.description } : {}),
      ...(choice.value === currentValue ? { isDefault: true } : {}),
    })),
    ...(nonEmpty(option.description) ? { description: nonEmpty(option.description) } : {}),
    ...(currentValue ? { currentValue } : {}),
  } satisfies SelectProviderOptionDescriptor;

  return {
    configId: option.id,
    values,
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
  const modelOption = findAcpConfigOption(configOptions, ACP_MODEL_CONFIG_ID);
  const selectModelOption = modelOption?.type === "select" ? modelOption : undefined;
  const modelChoices = selectModelOption
    ? uniqueOptions(flattenAcpSelectOptions(selectModelOption))
    : [];
  const modelValues = modelChoices.map((choice) => choice.value);
  const currentModel = selectModelOption
    ? currentValueIn(selectModelOption, modelValues)
    : undefined;
  const reasoningConfig = projectReasoningConfig(findReasoningConfigOption(configOptions));
  const capabilities = reasoningConfig ? { optionDescriptors: [reasoningConfig.descriptor] } : null;

  const models = modelChoices.map(
    (choice) =>
      ({
        slug: choice.value,
        name: choice.name,
        ...(choice.groupName ? { subProvider: choice.groupName } : {}),
        isCustom: false,
        ...(choice.value === currentModel ? { isDefault: true } : {}),
        capabilities,
      }) satisfies ServerProviderModel,
  );

  const modelConfig: ProjectedAcpSelectConfig | undefined = selectModelOption
    ? {
        configId: selectModelOption.id,
        values: modelValues,
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
  return { configId: config.configId, value: requestedValue };
}
