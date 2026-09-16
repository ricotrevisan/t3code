import { describe, expect, it } from "vite-plus/test";
import type * as AcpSchema from "effect-acp/schema";

import {
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
  it("flattens grouped and ungrouped ACP select options", () => {
    expect(flattenAcpSelectOptions(modelConfigOption)).toEqual([
      {
        value: '["control-fixture","beta"]',
        name: "Beta",
        description: "Stable control model",
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
        slug: '["other-fixture","alpha"]',
        name: "Alpha",
        subProvider: "Other Fixture",
        isCustom: false,
        capabilities: {
          optionDescriptors: [projected.reasoningConfig?.descriptor],
        },
      },
    ]);
    expect(projected.modelConfig).toEqual({
      configId: "model",
      currentValue: '["control-fixture","beta"]',
      values: ['["control-fixture","beta"]', '["other-fixture","alpha"]'],
    });
  });

  it("projects reasoning choices and omits the empty default", () => {
    const reasoning = projectAcpConfigOptions(dshConfigOptions).reasoningConfig;

    expect(reasoning).toEqual({
      configId: "reasoning_effort",
      values: ["low", "high"],
      descriptor: {
        id: "reasoningEffort",
        label: "Reasoning effort",
        type: "select",
        options: [
          { id: "low", label: "Low" },
          { id: "high", label: "High" },
        ],
      },
    });
  });

  it("falls back to the thought_level category but requires exact model id", () => {
    const projected = projectAcpConfigOptions([
      { ...modelConfigOption, id: "Model" },
      { ...reasoningConfigOption, id: "thinking", currentValue: "high" },
    ]);

    expect(projected.models).toEqual([]);
    expect(projected.modelConfig).toBeUndefined();
    expect(projected.reasoningConfig).toMatchObject({
      configId: "thinking",
      currentValue: "high",
      descriptor: {
        currentValue: "high",
        options: [
          { id: "low", label: "Low" },
          { id: "high", label: "High", isDefault: true },
        ],
      },
    });
    expect(findAcpConfigOption(dshConfigOptions, "Model")).toBeUndefined();
  });

  it("finds selections and validates config updates without changing tokens", () => {
    const projected = projectAcpConfigOptions(dshConfigOptions);
    const reasoning = findAcpReasoningSelection([
      { id: "unrelated", value: true },
      { id: "reasoningEffort", value: "high" },
    ]);

    expect(reasoning).toBe("high");
    expect(getAcpConfigOptionUpdate(projected.modelConfig, '["other-fixture","alpha"]')).toEqual({
      configId: "model",
      value: '["other-fixture","alpha"]',
    });
    expect(getAcpConfigOptionUpdate(projected.reasoningConfig, reasoning)).toEqual({
      configId: "reasoning_effort",
      value: "high",
    });
    expect(
      getAcpConfigOptionUpdate(projected.modelConfig, '["control-fixture","beta"]'),
    ).toBeUndefined();
    expect(getAcpConfigOptionUpdate(projected.reasoningConfig, "medium")).toBeUndefined();
  });
});
