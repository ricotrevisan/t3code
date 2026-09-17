"use client";

import { useEffect, useMemo, useState, type ReactNode } from "react";
import * as Option from "effect/Option";
import * as Schema from "effect/Schema";
import type {
  ProviderSettingsFormAnnotation,
  ProviderSettingsFormControl,
  ProviderSettingsFormOption,
  ProviderSettingsFormSchemaAnnotation,
  ProviderAdapterConfigSchema,
} from "@t3tools/contracts";

import { cn } from "../../lib/utils";
import { DraftInput } from "../ui/draft-input";
import { Input } from "../ui/input";
import { Select, SelectItem, SelectPopup, SelectTrigger, SelectValue } from "../ui/select";
import { Switch } from "../ui/switch";
import { Textarea } from "../ui/textarea";
import type { ProviderClientDefinition } from "./providerDriverMeta";
import { SettingsRow } from "./settingsLayout";
import {
  nextAdapterConfigWithFieldValue,
  normalizeAdapterConfigSchema,
  validateAdapterConfig,
  type AdapterConfigDefault,
  type AdapterConfigFieldModel,
} from "./providerAdapterConfig";

export interface ProviderSettingsFieldModel {
  readonly key: string;
  readonly control: ProviderSettingsFormControl;
  readonly label: string;
  readonly description?: string | undefined;
  readonly placeholder?: string | undefined;
  readonly clearWhenEmpty: "omit" | "persist";
  readonly defaultBooleanValue?: boolean | undefined;
  /** Choices for a `select` control. The first entry is the default. */
  readonly options?: ReadonlyArray<ProviderSettingsFormOption> | undefined;
}

function titleizeFieldKey(key: string): string {
  return key
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/[-_]+/g, " ")
    .replace(/^./, (char) => char.toUpperCase());
}

function readFieldAnnotations(
  fieldSchema: ProviderClientDefinition["settingsSchema"]["fields"][string],
) {
  return Schema.resolveAnnotationsKey(fieldSchema) ?? Schema.resolveAnnotations(fieldSchema);
}

function readFieldAnnotationString(
  fieldSchema: ProviderClientDefinition["settingsSchema"]["fields"][string],
  key: "title" | "description",
): string | undefined {
  const annotations = readFieldAnnotations(fieldSchema);
  const value = annotations?.[key];
  return typeof value === "string" ? value : undefined;
}

function readProviderSettingsFormAnnotation(
  fieldSchema: ProviderClientDefinition["settingsSchema"]["fields"][string],
): ProviderSettingsFormAnnotation {
  const annotation = readFieldAnnotations(fieldSchema)?.providerSettingsForm;
  return annotation ?? {};
}

function readProviderSettingsFormSchemaAnnotation(
  definition: ProviderClientDefinition,
): ProviderSettingsFormSchemaAnnotation {
  return Schema.resolveAnnotations(definition.settingsSchema)?.providerSettingsFormSchema ?? {};
}

function readFieldBooleanDefault(
  fieldSchema: ProviderClientDefinition["settingsSchema"]["fields"][string],
): boolean | undefined {
  const decodeDefault = Schema.decodeUnknownOption(fieldSchema as Schema.Decoder<unknown>);
  const decoded = decodeDefault(undefined);
  return Option.isSome(decoded) && typeof decoded.value === "boolean" ? decoded.value : undefined;
}

export function deriveProviderSettingsFields(
  definition: ProviderClientDefinition,
): ReadonlyArray<ProviderSettingsFieldModel> {
  const schemaAnnotation = readProviderSettingsFormSchemaAnnotation(definition);
  const orderedKeys = new Map(
    (schemaAnnotation.order ?? []).map((key, index) => [key, index] as const),
  );
  const orderFallbackOffset = orderedKeys.size;

  return Object.keys(definition.settingsSchema.fields)
    .map((key, index) => ({ key, index }))
    .toSorted((left, right) => {
      return (
        (orderedKeys.get(left.key) ?? orderFallbackOffset + left.index) -
        (orderedKeys.get(right.key) ?? orderFallbackOffset + right.index)
      );
    })
    .flatMap(({ key }) => {
      const fieldSchema = definition.settingsSchema.fields[key]!;
      const formAnnotation = readProviderSettingsFormAnnotation(fieldSchema);
      if (formAnnotation.hidden) return [];

      const annotatedTitle = readFieldAnnotationString(fieldSchema, "title");
      const annotatedDescription = readFieldAnnotationString(fieldSchema, "description");
      return [
        {
          key,
          control: formAnnotation.control ?? "text",
          label: annotatedTitle ?? titleizeFieldKey(key),
          ...(annotatedDescription !== undefined ? { description: annotatedDescription } : {}),
          ...(formAnnotation.placeholder !== undefined
            ? { placeholder: formAnnotation.placeholder }
            : {}),
          clearWhenEmpty: formAnnotation.clearWhenEmpty ?? "omit",
          ...(formAnnotation.control === "switch"
            ? { defaultBooleanValue: readFieldBooleanDefault(fieldSchema) }
            : {}),
          ...(formAnnotation.control === "select" && formAnnotation.options
            ? { options: formAnnotation.options }
            : {}),
        } satisfies ProviderSettingsFieldModel,
      ];
    });
}

export function readProviderConfigString(config: unknown, key: string): string {
  if (config === null || typeof config !== "object") return "";
  const value = (config as Record<string, unknown>)[key];
  return typeof value === "string" ? value : "";
}

export function readProviderConfigBoolean(
  config: unknown,
  key: string,
  defaultValue = false,
): boolean {
  if (config === null || typeof config !== "object") return defaultValue;
  const value = (config as Record<string, unknown>)[key];
  return typeof value === "boolean" ? value : defaultValue;
}

export function nextProviderConfigWithFieldValue(
  config: unknown,
  field: ProviderSettingsFieldModel,
  value: string | boolean,
): Record<string, unknown> | undefined {
  const base: Record<string, unknown> =
    config !== null && typeof config === "object" ? { ...(config as Record<string, unknown>) } : {};

  if (typeof value === "boolean") {
    const emptyBooleanValue = field.defaultBooleanValue ?? false;
    if (field.clearWhenEmpty === "omit" && value === emptyBooleanValue) {
      delete base[field.key];
    } else {
      base[field.key] = value;
    }
    return Object.keys(base).length > 0 ? base : undefined;
  }

  const trimmed = value.trim();
  if (field.clearWhenEmpty === "omit" && trimmed.length === 0) {
    delete base[field.key];
  } else {
    base[field.key] = value;
  }
  return Object.keys(base).length > 0 ? base : undefined;
}

interface ProviderSettingsFormProps {
  readonly definition: ProviderClientDefinition;
  readonly value: unknown;
  readonly idPrefix: string;
  /**
   * `card` stacks label over control, `dialog` is the compact wizard layout,
   * and `settings` renders the shared settings row treatment.
   */
  readonly variant: "card" | "dialog" | "settings";
  readonly onChange: (nextConfig: Record<string, unknown> | undefined) => void;
}

/** Stores the default choice as an omitted key so unchanged configs stay small. */
function ProviderSettingsSelect({
  field,
  value,
  inputId,
  size,
  className,
  onChange,
}: {
  readonly field: ProviderSettingsFieldModel;
  readonly value: unknown;
  readonly inputId: string;
  readonly size: "sm" | "xs";
  readonly className?: string | undefined;
  readonly onChange: ProviderSettingsFormProps["onChange"];
}) {
  const options = field.options ?? [];
  const fallback = options[0]?.value ?? "";
  const current = readProviderConfigString(value, field.key) || fallback;
  const label = options.find((option) => option.value === current)?.label ?? current;
  return (
    <Select
      value={current}
      onValueChange={(next) => {
        if (typeof next !== "string") return;
        onChange(nextProviderConfigWithFieldValue(value, field, next === fallback ? "" : next));
      }}
    >
      <SelectTrigger id={inputId} size={size} className={className} aria-label={field.label}>
        <SelectValue>{label}</SelectValue>
      </SelectTrigger>
      <SelectPopup align="start" alignItemWithTrigger={false}>
        {options.map((option) => (
          <SelectItem key={option.value} value={option.value}>
            {option.label}
          </SelectItem>
        ))}
      </SelectPopup>
    </Select>
  );
}

function FieldFrame(props: {
  readonly variant: ProviderSettingsFormProps["variant"];
  readonly children: ReactNode;
}) {
  if (props.variant === "card") {
    return <div>{props.children}</div>;
  }
  return <div className="grid gap-1.5">{props.children}</div>;
}

interface ProviderSettingsFieldRowProps {
  readonly field: ProviderSettingsFieldModel;
  readonly value: unknown;
  readonly idPrefix: string;
  readonly variant: ProviderSettingsFormProps["variant"];
  readonly onChange: ProviderSettingsFormProps["onChange"];
}

function ProviderSettingsFieldRow({
  field,
  value,
  idPrefix,
  variant,
  onChange,
}: ProviderSettingsFieldRowProps) {
  const inputId = `${idPrefix}-${field.key}`;
  const descriptionClassName =
    variant === "dialog"
      ? "text-[11px] text-muted-foreground"
      : "mt-1 block text-xs text-muted-foreground";
  const label = <span className="text-xs font-medium text-foreground">{field.label}</span>;
  const description = field.description ? (
    <span className={descriptionClassName}>{field.description}</span>
  ) : null;

  if (variant === "settings") {
    const descriptionId = field.description ? `${inputId}-description` : undefined;
    const control =
      field.control === "switch" ? (
        <Switch
          checked={readProviderConfigBoolean(value, field.key, field.defaultBooleanValue)}
          onCheckedChange={(checked) =>
            onChange(nextProviderConfigWithFieldValue(value, field, Boolean(checked)))
          }
          aria-label={field.label}
          aria-describedby={descriptionId}
        />
      ) : field.control === "select" ? (
        <ProviderSettingsSelect
          field={field}
          value={value}
          inputId={inputId}
          size="sm"
          className="w-full max-w-full @min-[32rem]/settings-row:w-56"
          onChange={onChange}
        />
      ) : field.control === "textarea" ? (
        <Textarea
          id={inputId}
          aria-describedby={descriptionId}
          className="w-full max-w-full @min-[32rem]/settings-row:w-[min(24rem,50cqw)]"
          value={readProviderConfigString(value, field.key)}
          onChange={(event) =>
            onChange(nextProviderConfigWithFieldValue(value, field, event.target.value))
          }
          placeholder={field.placeholder}
          spellCheck={false}
        />
      ) : (
        <DraftInput
          id={inputId}
          aria-describedby={descriptionId}
          size="sm"
          className="w-full max-w-full @min-[32rem]/settings-row:w-56"
          type={field.control === "password" ? "password" : undefined}
          autoComplete={field.control === "password" ? "off" : undefined}
          value={readProviderConfigString(value, field.key)}
          onCommit={(next) => onChange(nextProviderConfigWithFieldValue(value, field, next))}
          placeholder={field.placeholder}
          spellCheck={false}
        />
      );

    return (
      <SettingsRow
        title={
          field.control === "switch" ? field.label : <label htmlFor={inputId}>{field.label}</label>
        }
        description={
          field.description ? <span id={descriptionId}>{field.description}</span> : undefined
        }
        control={control}
      />
    );
  }

  if (field.control === "switch") {
    return (
      <FieldFrame variant={variant}>
        <div className="flex items-center justify-between gap-3">
          <div className="min-w-0">
            {label}
            {description}
          </div>
          <Switch
            checked={readProviderConfigBoolean(value, field.key, field.defaultBooleanValue)}
            onCheckedChange={(checked) =>
              onChange(nextProviderConfigWithFieldValue(value, field, Boolean(checked)))
            }
            aria-label={field.label}
          />
        </div>
      </FieldFrame>
    );
  }

  if (field.control === "select") {
    return (
      <FieldFrame variant={variant}>
        <label htmlFor={inputId} className={cn(variant === "card" && "block")}>
          {label}
          <ProviderSettingsSelect
            field={field}
            value={value}
            inputId={inputId}
            size="sm"
            className={cn("w-full", variant === "card" && "mt-1.5")}
            onChange={onChange}
          />
          {description}
        </label>
      </FieldFrame>
    );
  }

  if (field.control === "textarea") {
    return (
      <FieldFrame variant={variant}>
        <label htmlFor={inputId} className={cn(variant === "card" && "block")}>
          {label}
          <Textarea
            id={inputId}
            className={cn(variant === "card" && "mt-1.5")}
            value={readProviderConfigString(value, field.key)}
            onChange={(event) =>
              onChange(nextProviderConfigWithFieldValue(value, field, event.target.value))
            }
            placeholder={field.placeholder}
            spellCheck={false}
          />
          {description}
        </label>
      </FieldFrame>
    );
  }

  const type = field.control === "password" ? "password" : undefined;
  return (
    <FieldFrame variant={variant}>
      <label htmlFor={inputId} className={cn(variant === "card" && "block")}>
        {label}
        {variant === "card" ? (
          <DraftInput
            id={inputId}
            size="sm"
            className="mt-1.5"
            type={type}
            autoComplete={field.control === "password" ? "off" : undefined}
            value={readProviderConfigString(value, field.key)}
            onCommit={(next) => onChange(nextProviderConfigWithFieldValue(value, field, next))}
            placeholder={field.placeholder}
            spellCheck={false}
          />
        ) : (
          <Input
            id={inputId}
            className="bg-background"
            type={type}
            autoComplete={field.control === "password" ? "off" : undefined}
            value={readProviderConfigString(value, field.key)}
            onChange={(event) =>
              onChange(nextProviderConfigWithFieldValue(value, field, event.target.value))
            }
            placeholder={field.placeholder}
            spellCheck={false}
          />
        )}
        {description}
      </label>
    </FieldFrame>
  );
}

export function ProviderSettingsForm({
  definition,
  value,
  idPrefix,
  variant,
  onChange,
}: ProviderSettingsFormProps) {
  const fields = useMemo(() => deriveProviderSettingsFields(definition), [definition]);

  if (fields.length === 0) {
    return null;
  }

  return (
    <>
      {fields.map((field) => (
        <ProviderSettingsFieldRow
          key={`${idPrefix}:${field.key}`}
          field={field}
          value={value}
          idPrefix={idPrefix}
          variant={variant}
          onChange={onChange}
        />
      ))}
    </>
  );
}

interface AdapterConfigFormProps {
  readonly schema: ProviderAdapterConfigSchema;
  readonly value: unknown;
  readonly idPrefix: string;
  readonly variant: "card" | "dialog";
  readonly showErrors?: boolean;
  readonly onChange: (nextConfig: Record<string, unknown> | undefined) => void;
}

function readAdapterConfigValue(value: unknown, key: string): unknown {
  if (value === null || typeof value !== "object" || Array.isArray(value)) return undefined;
  return (value as Record<string, unknown>)[key];
}

function adapterTextValue(field: AdapterConfigFieldModel, value: unknown): string {
  const current = readAdapterConfigValue(value, field.key);
  if (field.kind === "string-array") {
    return Array.isArray(current) && current.every((entry) => typeof entry === "string")
      ? current.join("\n")
      : "";
  }
  if (typeof current === "string" || typeof current === "number") return String(current);
  return "";
}

export function readAdapterNumberDraftValue(value: unknown): string {
  return typeof value === "string" || typeof value === "number" ? String(value) : "";
}

function AdapterConfigFieldRow({
  field,
  value,
  idPrefix,
  variant,
  error,
  onChange,
}: {
  readonly field: AdapterConfigFieldModel;
  readonly value: unknown;
  readonly idPrefix: string;
  readonly variant: AdapterConfigFormProps["variant"];
  readonly error: string | undefined;
  readonly onChange: AdapterConfigFormProps["onChange"];
}) {
  const inputId = `${idPrefix}-${field.key}`;
  const current = readAdapterConfigValue(value, field.key);
  const [draftError, setDraftError] = useState<string | undefined>();
  const [numberDraft, setNumberDraft] = useState(() => readAdapterNumberDraftValue(current));
  useEffect(() => {
    setNumberDraft(readAdapterNumberDraftValue(current));
    setDraftError(undefined);
  }, [current, field.defaultValue, field.kind, idPrefix]);
  const visibleError = draftError ?? error;
  const descriptionClassName =
    variant === "card"
      ? "mt-1 block text-xs text-muted-foreground"
      : "text-[11px] text-muted-foreground";
  const publish = (next: AdapterConfigDefault | undefined) => {
    setDraftError(undefined);
    onChange(nextAdapterConfigWithFieldValue(value, field, next));
  };
  const label = (
    <span className="text-xs font-medium text-foreground">
      {field.label}
      {field.required ? <span className="text-destructive"> *</span> : null}
    </span>
  );
  const detail = (
    <>
      {field.description ? <span className={descriptionClassName}>{field.description}</span> : null}
      {visibleError ? (
        <span className="mt-1 block text-xs text-destructive">{visibleError}</span>
      ) : null}
    </>
  );

  if (field.kind === "unsupported") {
    return (
      <div className="rounded-md border border-dashed border-border/70 bg-muted/20 p-3">
        {label}
        {field.description ? (
          <span className={descriptionClassName}>{field.description}</span>
        ) : null}
        <span className="mt-1 block text-xs text-muted-foreground">{field.unsupportedReason}</span>
        {visibleError ? (
          <span className="mt-1 block text-xs text-destructive">{visibleError}</span>
        ) : null}
      </div>
    );
  }

  if (field.kind === "boolean") {
    const checked =
      typeof current === "boolean"
        ? current
        : typeof field.defaultValue === "boolean"
          ? field.defaultValue
          : false;
    return (
      <FieldFrame variant={variant}>
        <div className="flex items-center justify-between gap-3">
          <div className="min-w-0">
            {label}
            {detail}
          </div>
          <Switch
            checked={checked}
            onCheckedChange={(next) => publish(Boolean(next))}
            aria-label={field.label}
          />
        </div>
      </FieldFrame>
    );
  }

  if (field.kind === "string-enum") {
    const selected = typeof current === "string" && field.options?.includes(current) ? current : "";
    return (
      <FieldFrame variant={variant}>
        <label htmlFor={inputId} className="grid gap-1.5">
          {label}
          <select
            id={inputId}
            className="h-8 rounded-md border border-input bg-background px-2 text-sm text-foreground"
            value={selected}
            aria-invalid={visibleError !== undefined}
            onChange={(event) => publish(event.target.value || undefined)}
          >
            <option value="">Select a value</option>
            {field.options?.map((option) => (
              <option key={option} value={option}>
                {option}
              </option>
            ))}
          </select>
          {detail}
        </label>
      </FieldFrame>
    );
  }

  const commitText = (next: string) => {
    if (field.kind === "number" || field.kind === "integer") setNumberDraft(next);
    if (field.kind === "string") {
      publish(next.length > 0 ? next : undefined);
      return;
    }
    if (field.kind === "string-array") {
      publish(next.length > 0 ? next.split("\n") : undefined);
      return;
    }
    const trimmed = next.trim();
    if (trimmed.length === 0) {
      publish(undefined);
      return;
    }
    const number = Number(trimmed);
    if (!Number.isFinite(number) || (field.kind === "integer" && !Number.isInteger(number))) {
      setDraftError(
        field.kind === "integer"
          ? `${field.label} must be a whole number.`
          : `${field.label} must be a number.`,
      );
      return;
    }
    publish(number);
  };

  const textValue =
    field.kind === "number" || field.kind === "integer"
      ? numberDraft
      : adapterTextValue(field, value);
  if (field.kind === "string-array") {
    return (
      <FieldFrame variant={variant}>
        <label htmlFor={inputId} className="grid gap-1.5">
          {label}
          <Textarea
            id={inputId}
            value={textValue}
            aria-invalid={visibleError !== undefined}
            onChange={(event) => commitText(event.target.value)}
            placeholder="One value per line"
            spellCheck={false}
          />
          {detail}
        </label>
      </FieldFrame>
    );
  }

  return (
    <FieldFrame variant={variant}>
      <label htmlFor={inputId} className={cn(variant === "card" && "block")}>
        {label}
        {variant === "card" ? (
          <DraftInput
            id={inputId}
            className="mt-1.5"
            value={textValue}
            inputMode={field.kind === "number" || field.kind === "integer" ? "decimal" : undefined}
            aria-invalid={visibleError !== undefined}
            onCommit={commitText}
            spellCheck={false}
          />
        ) : (
          <Input
            id={inputId}
            className="bg-background"
            value={textValue}
            inputMode={field.kind === "number" || field.kind === "integer" ? "decimal" : undefined}
            aria-invalid={visibleError !== undefined}
            onChange={(event) => commitText(event.target.value)}
            spellCheck={false}
          />
        )}
        {detail}
      </label>
    </FieldFrame>
  );
}

/** Schema-driven form for trusted adapter manifests. It never interprets password formats. */
export function AdapterConfigForm({
  schema,
  value,
  idPrefix,
  variant,
  showErrors = true,
  onChange,
}: AdapterConfigFormProps) {
  const normalized = useMemo(() => normalizeAdapterConfigSchema(schema), [schema]);
  const errors = useMemo(() => validateAdapterConfig(normalized, value), [normalized, value]);

  if (normalized.unsupportedRoot) {
    return (
      <div className="grid gap-1">
        <p className="text-xs text-muted-foreground">
          This adapter uses a configuration schema that this client cannot edit. Existing values are
          preserved.
        </p>
        {showErrors && errors.$root ? (
          <p className="text-xs text-destructive">{errors.$root}</p>
        ) : null}
      </div>
    );
  }

  if (normalized.fields.length === 0) return null;

  return (
    <>
      {normalized.fields.map((field) => (
        <AdapterConfigFieldRow
          key={`${idPrefix}:${field.key}`}
          field={field}
          value={value}
          idPrefix={idPrefix}
          variant={variant}
          error={showErrors ? errors[field.key] : undefined}
          onChange={onChange}
        />
      ))}
    </>
  );
}
