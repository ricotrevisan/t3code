import { describe, expect, it } from "vite-plus/test";
import type * as AcpSchema from "effect-acp/schema";

import {
  ACP_EMPTY_VALUE_SENTINEL,
  findAcpConfigOption,
  findAcpReasoningSelection,
  flattenAcpSelectOptions,
  getAcpConfigOptionUpdate,
  projectAcpConfigOptions,
} from "./AcpConfigOptions.ts";

type SelectConfigOption = Extract<AcpSchema.SessionConfigOption, { readonly type: "select" }>;

const modelConfigOption = {
  id: "model",
  name: "Model",
  category: "model",
  type: "select",
  currentValue: '["control-fixture","beta"]',
  options: [
    {
      group: "control-fixture",
      name: "Control Fixture",
      options: [
        {
          value: '["control-fixture","beta"]',
          name: "Beta",
          description: "Stable control model",
        },
        { value: "", name: "Agent default" },
      ],
    },
    {
      group: "other-fixture",
      name: "Other Fixture",
      options: [{ value: '["other-fixture","alpha"]', name: "Alpha" }],
    },
  ],
} satisfies SelectConfigOption;

const reasoningConfigOption = {
  id: "reasoning_effort",
  name: "Reasoning effort",
  category: "thought_level",
  type: "select",
  currentValue: "",
  options: [
    { value: "", name: "Default" },
    { value: "low", name: "Low" },
    { value: "high", name: "High" },
  ],
} satisfies SelectConfigOption;

const dshConfigOptions: ReadonlyArray<AcpSchema.SessionConfigOption> = [
  modelConfigOption,
  reasoningConfigOption,
];

describe("ACP config option projection", () => {
  it("flattens grouped and ungrouped ACP select options without dropping empty values", () => {
    expect(flattenAcpSelectOptions(modelConfigOption)).toEqual([
      {
        value: '["control-fixture","beta"]',
        name: "Beta",
        description: "Stable control model",
        groupId: "control-fixture",
        groupName: "Control Fixture",
      },
      {
        value: "",
        name: "Agent default",
        groupId: "control-fixture",
        groupName: "Control Fixture",
      },
      {
        value: '["other-fixture","alpha"]',
        name: "Alpha",
        groupId: "other-fixture",
        groupName: "Other Fixture",
      },
    ]);
    expect(flattenAcpSelectOptions(reasoningConfigOption)).toEqual([
      { value: "", name: "Default" },
      { value: "low", name: "Low" },
      { value: "high", name: "High" },
    ]);
  });

  it("preserves opaque model values and projects group labels", () => {
    const projected = projectAcpConfigOptions(dshConfigOptions);

    expect(projected.models).toEqual([
      {
        slug: '["control-fixture","beta"]',
        name: "Beta",
        subProvider: "Control Fixture",
        isCustom: false,
        isDefault: true,
        capabilities: {
          optionDescriptors: [projected.reasoningConfig?.descriptor],
        },
      },
      {
        slug: ACP_EMPTY_VALUE_SENTINEL,
        name: "Agent default",
        subProvider: "Control Fixture",
        isCustom: false,
        capabilities: {
          optionDescriptors: [projected.reasoningConfig?.descriptor],
        },
      },
      {
        slug: '["other-fixture","alpha"]',
        name: "Alpha",
        subProvider: "Other Fixture",
        isCustom: false,
        capabilities: {
          optionDescriptors: [projected.reasoningConfig?.descriptor],
        },
      },
    ]);
    expect(projected.modelConfig).toMatchObject({
      configId: "model",
      currentValue: '["control-fixture","beta"]',
      values: ['["control-fixture","beta"]', ACP_EMPTY_VALUE_SENTINEL, '["other-fixture","alpha"]'],
    });
    expect(projected.modelConfig?.canonicalToWireValue.get(ACP_EMPTY_VALUE_SENTINEL)).toBe("");
    expect(projected.modelConfig?.wireToCanonicalValue.get("")).toBe(ACP_EMPTY_VALUE_SENTINEL);
  });

  it("projects an empty reasoning default through the canonical sentinel", () => {
    const reasoning = projectAcpConfigOptions(dshConfigOptions).reasoningConfig;

    expect(reasoning).toMatchObject({
      configId: "reasoning_effort",
      currentValue: ACP_EMPTY_VALUE_SENTINEL,
      values: [ACP_EMPTY_VALUE_SENTINEL, "low", "high"],
      descriptor: {
        id: "reasoningEffort",
        label: "Reasoning effort",
        type: "select",
        currentValue: ACP_EMPTY_VALUE_SENTINEL,
        options: [
          { id: ACP_EMPTY_VALUE_SENTINEL, label: "Default", isDefault: true },
          { id: "low", label: "Low" },
          { id: "high", label: "High" },
        ],
      },
    });
    expect(reasoning?.canonicalToWireValue.get(ACP_EMPTY_VALUE_SENTINEL)).toBe("");
    expect(reasoning?.wireToCanonicalValue.get("")).toBe(ACP_EMPTY_VALUE_SENTINEL);
  });

  it("prefers the standard model category with an opaque id and falls back to exact id model", () => {
    const categorized = projectAcpConfigOptions([
      { ...modelConfigOption, id: "model", category: "other" },
      {
        ...modelConfigOption,
        id: "llm",
        currentValue: '["other-fixture","alpha"]',
      },
    ]);

    expect(categorized.modelConfig).toMatchObject({
      configId: "llm",
      currentValue: '["other-fixture","alpha"]',
    });

    const fallback = projectAcpConfigOptions([{ ...modelConfigOption, category: "other" }]);
    expect(fallback.modelConfig?.configId).toBe("model");
    expect(findAcpConfigOption(dshConfigOptions, "Model")).toBeUndefined();
  });

  it("prefers the thought_level category and falls back to exact id reasoning_effort", () => {
    const categorized = projectAcpConfigOptions([
      { ...reasoningConfigOption, category: "other", currentValue: "high" },
      { ...reasoningConfigOption, id: "thinking", currentValue: "low" },
    ]);
    expect(categorized.reasoningConfig).toMatchObject({
      configId: "thinking",
      currentValue: "low",
      descriptor: {
        currentValue: "low",
        options: [
          { id: ACP_EMPTY_VALUE_SENTINEL, label: "Default" },
          { id: "low", label: "Low", isDefault: true },
          { id: "high", label: "High" },
        ],
      },
    });

    const fallback = projectAcpConfigOptions([
      { ...reasoningConfigOption, category: "other", currentValue: "high" },
    ]);
    expect(fallback.reasoningConfig).toMatchObject({
      configId: "reasoning_effort",
      currentValue: "high",
    });
  });

  it("round-trips the DSH empty model default after choosing another model", () => {
    const emptyDefault = projectAcpConfigOptions([
      { ...modelConfigOption, currentValue: "" },
    ]).modelConfig;
    expect(emptyDefault?.currentValue).toBe(ACP_EMPTY_VALUE_SENTINEL);
    expect(
      projectAcpConfigOptions([{ ...modelConfigOption, currentValue: "" }]).models.find(
        (model) => model.slug === ACP_EMPTY_VALUE_SENTINEL,
      ),
    ).toMatchObject({ isDefault: true });
    expect(getAcpConfigOptionUpdate(emptyDefault, '["other-fixture","alpha"]')).toEqual({
      configId: "model",
      value: '["other-fixture","alpha"]',
    });

    const chosen = projectAcpConfigOptions([
      { ...modelConfigOption, currentValue: '["other-fixture","alpha"]' },
    ]).modelConfig;
    expect(getAcpConfigOptionUpdate(chosen, ACP_EMPTY_VALUE_SENTINEL)).toEqual({
      configId: "model",
      value: "",
    });
  });

  it("chooses collision-free sentinels for every value T3 cannot round-trip", () => {
    const options = [
      { value: "", name: "Agent default" },
      { value: ACP_EMPTY_VALUE_SENTINEL, name: "Literal sentinel" },
      { value: `${ACP_EMPTY_VALUE_SENTINEL}:1`, name: "Literal suffixed sentinel" },
      { value: "opaque:model/value", name: "Opaque value" },
      { value: "   ", name: "Whitespace only" },
      { value: " padded opaque value ", name: "Padded opaque value" },
    ];
    const projected = projectAcpConfigOptions([
      {
        ...modelConfigOption,
        id: "llm",
        currentValue: " padded opaque value ",
        options,
      },
    ]);
    const emptyCanonicalValue = `${ACP_EMPTY_VALUE_SENTINEL}:2`;
    const whitespaceCanonicalValue = `${ACP_EMPTY_VALUE_SENTINEL}:3`;
    const paddedCanonicalValue = `${ACP_EMPTY_VALUE_SENTINEL}:4`;

    expect(projected.models.map((model) => model.slug)).toEqual([
      emptyCanonicalValue,
      ACP_EMPTY_VALUE_SENTINEL,
      `${ACP_EMPTY_VALUE_SENTINEL}:1`,
      "opaque:model/value",
      whitespaceCanonicalValue,
      paddedCanonicalValue,
    ]);
    expect(projected.modelConfig?.currentValue).toBe(paddedCanonicalValue);
    expect(projected.models.find((model) => model.slug === paddedCanonicalValue)).toMatchObject({
      isDefault: true,
    });
    expect(projected.modelConfig?.canonicalToWireValue.get(emptyCanonicalValue)).toBe("");
    expect(projected.modelConfig?.canonicalToWireValue.get(whitespaceCanonicalValue)).toBe("   ");
    expect(projected.modelConfig?.canonicalToWireValue.get(paddedCanonicalValue)).toBe(
      " padded opaque value ",
    );
    expect(projected.modelConfig?.canonicalToWireValue.get("opaque:model/value")).toBe(
      "opaque:model/value",
    );
    const safeCurrent = projectAcpConfigOptions([
      {
        ...modelConfigOption,
        id: "llm",
        currentValue: "opaque:model/value",
        options,
      },
    ]).modelConfig;
    expect(getAcpConfigOptionUpdate(safeCurrent, paddedCanonicalValue)).toEqual({
      configId: "llm",
      value: " padded opaque value ",
    });
    expect(getAcpConfigOptionUpdate(projected.modelConfig, whitespaceCanonicalValue)).toEqual({
      configId: "llm",
      value: "   ",
    });
    expect(getAcpConfigOptionUpdate(projected.modelConfig, emptyCanonicalValue)).toEqual({
      configId: "llm",
      value: "",
    });
    expect(getAcpConfigOptionUpdate(projected.modelConfig, ACP_EMPTY_VALUE_SENTINEL)).toEqual({
      configId: "llm",
      value: ACP_EMPTY_VALUE_SENTINEL,
    });
  });

  it("resets reasoning to its empty wire value", () => {
    const chosen = projectAcpConfigOptions([
      { ...reasoningConfigOption, currentValue: "high" },
    ]).reasoningConfig;
    const resetSelection = findAcpReasoningSelection([
      { id: "unrelated", value: true },
      { id: "reasoningEffort", value: ACP_EMPTY_VALUE_SENTINEL },
    ]);

    expect(resetSelection).toBe(ACP_EMPTY_VALUE_SENTINEL);
    expect(getAcpConfigOptionUpdate(chosen, resetSelection)).toEqual({
      configId: "reasoning_effort",
      value: "",
    });
    expect(getAcpConfigOptionUpdate(chosen, "medium")).toBeUndefined();
  });
});
