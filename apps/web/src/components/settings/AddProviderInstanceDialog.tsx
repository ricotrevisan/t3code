"use client";

import { Radio as RadioPrimitive } from "@base-ui/react/radio";
import { CheckIcon, PackageIcon } from "lucide-react";
import { useMemo, useState } from "react";
import {
  ProviderInstanceId,
  ProviderDriverKind,
  type EnvironmentId,
  type ProviderAdapterManifestV1,
  type ProviderInstanceConfig,
} from "@t3tools/contracts";

import { useEnvironmentSettings, useUpdateEnvironmentSettings } from "../../hooks/useSettings";
import { cn } from "../../lib/utils";
import { normalizeProviderAccentColor } from "../../providerInstances";
import { Button } from "../ui/button";
import { ACPRegistryIcon, Gemini, GithubCopilotIcon, PiAgentIcon, type Icon } from "../Icons";
import { Dialog } from "../ui/dialog";
import { Badge } from "../ui/badge";
import { Input } from "../ui/input";
import { RadioGroup } from "../ui/radio-group";
import { toastManager } from "../ui/toast";
import { DRIVER_OPTION_BY_VALUE, DRIVER_OPTIONS } from "./providerDriverMeta";
import { ProviderAccentColorPicker } from "./ProviderAccentColorPicker";
import {
  AdapterConfigForm,
  ProviderSettingsForm,
  deriveProviderSettingsFields,
} from "./ProviderSettingsForm";
import { WizardPanel, WizardPopup, WizardHeader, WizardFooter } from "../ui/wizard";
import {
  ADD_PROVIDER_WIZARD_STEPS,
  resolveWizardNavigation,
  type WizardNavigation,
} from "./AddProviderInstanceDialog.logic";
import { AddProviderInstanceWizardSteps } from "./AddProviderInstanceWizardSteps";
import {
  addableAdapterManifests,
  adapterManifestSelectionKey,
  buildProviderInstanceConfig,
  defaultAdapterConfig,
  normalizeAdapterConfigSchema,
  resolveAdapterSelection,
  selectionKeyForBuiltInDriver,
  validateAdapterConfig,
} from "./providerAdapterConfig";

const PROVIDER_ACCENT_SWATCHES = [
  "#2563eb",
  "#16a34a",
  "#ea580c",
  "#dc2626",
  "#7c3aed",
  "#0891b2",
] as const;

/**
 * Normalize a user-provided label into a slug suffix for the instance id.
 * The full id is formed by prefixing the driver slug — e.g. label "Work" on
 * driver "codex" becomes `codex_work`. Output is trimmed to 48 chars so the
 * final composed id stays under the 64-char slug cap enforced by
 * `ProviderInstanceId` in `@t3tools/contracts`.
 */
function slugifyLabel(value: string): string {
  return value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "")
    .slice(0, 48);
}

function deriveInstanceId(driver: ProviderDriverKind, label: string): string {
  const slug = slugifyLabel(label);
  return slug ? `${driver}_${slug}` : "";
}

const INSTANCE_ID_PATTERN = /^[a-zA-Z][a-zA-Z0-9_-]*$/;
const DEFAULT_DRIVER_KIND = ProviderDriverKind.make("codex");
const EMPTY_ADAPTER_MANIFESTS: ReadonlyArray<ProviderAdapterManifestV1> = [];
interface ComingSoonDriverOption {
  readonly value: ProviderDriverKind;
  readonly label: string;
  readonly icon: Icon;
}

const COMING_SOON_DRIVER_OPTIONS: readonly ComingSoonDriverOption[] = [
  {
    value: ProviderDriverKind.make("githubCopilot"),
    label: "Github Copilot",
    icon: GithubCopilotIcon,
  },
  {
    value: ProviderDriverKind.make("gemini"),
    label: "Gemini",
    icon: Gemini,
  },
  {
    value: ProviderDriverKind.make("acpRegistry"),
    label: "ACP Registry",
    icon: ACPRegistryIcon,
  },
  {
    value: ProviderDriverKind.make("piAgent"),
    label: "Pi Agent",
    icon: PiAgentIcon,
  },
];

/**
 * Validate an instance id against the same slug rules the server applies in
 * `ProviderInstanceId` (see `packages/contracts/src/providerInstance.ts`).
 * Returns a user-facing error string, or `null` if valid.
 */
function validateInstanceId(id: string, existing: ReadonlySet<string>): string | null {
  if (id.length === 0) return "Instance ID is required.";
  if (id.length > 64) return "Instance ID must be 64 characters or fewer.";
  if (!INSTANCE_ID_PATTERN.test(id)) {
    return "Instance ID must start with a letter and use only letters, digits, '-', or '_'.";
  }
  if (existing.has(id)) return `An instance named '${id}' already exists.`;
  return null;
}

interface AddProviderInstanceDialogProps {
  readonly open: boolean;
  readonly environmentId: EnvironmentId;
  readonly environmentLabel: string;
  readonly adapterManifests?: ReadonlyArray<ProviderAdapterManifestV1>;
  readonly onOpenChange: (open: boolean) => void;
}

export function AddProviderInstanceDialog({
  open,
  environmentId,
  environmentLabel,
  adapterManifests = EMPTY_ADAPTER_MANIFESTS,
  onOpenChange,
}: AddProviderInstanceDialogProps) {
  const settings = useEnvironmentSettings(environmentId);
  const updateSettings = useUpdateEnvironmentSettings(environmentId);

  const [wizardStep, setWizardStep] = useState(0);
  const [selectionKey, setSelectionKey] = useState(() =>
    selectionKeyForBuiltInDriver(DEFAULT_DRIVER_KIND, adapterManifests),
  );
  const [label, setLabel] = useState("");
  const [accentColor, setAccentColor] = useState<string>("");
  const [instanceIdOverride, setInstanceIdOverride] = useState<string | null>(null);
  // Drafts are keyed by the exact choice, not only by driver. This keeps two
  // installed versions of the same adapter package from sharing config.
  const [configBySelection, setConfigBySelection] = useState<Record<string, unknown>>({});
  // Errors are suppressed until the user has tried to submit once. After that
  // they update live so fixing the problem clears the message in place.
  const [hasAttemptedSubmit, setHasAttemptedSubmit] = useState(false);

  const existingIds = useMemo(
    () => new Set(Object.keys(settings.providerInstances ?? {})),
    [settings.providerInstances],
  );
  const builtInDriverValues = useMemo(() => DRIVER_OPTIONS.map((option) => option.value), []);
  const builtInDrivers = useMemo(
    () => new Set(builtInDriverValues.map(String)),
    [builtInDriverValues],
  );
  const installedAdapterManifests = useMemo(
    () => addableAdapterManifests(adapterManifests, builtInDrivers),
    [adapterManifests, builtInDrivers],
  );
  const selection = resolveAdapterSelection(selectionKey, builtInDriverValues, adapterManifests);
  const selectedManifest =
    selection.kind === "manifest"
      ? selection.manifest
      : selection.kind === "built-in"
        ? selection.manifest
        : undefined;
  const selectionUnavailable = selection.kind === "missing";
  const driver =
    selection.kind === "manifest"
      ? selection.manifest.driver
      : selection.kind === "built-in"
        ? selection.driver
        : undefined;
  const driverOption = driver === undefined ? undefined : DRIVER_OPTION_BY_VALUE[driver];
  const selectedLabel =
    selectedManifest?.displayName ?? driverOption?.label ?? "Unavailable adapter";
  const instanceId =
    instanceIdOverride ?? (driver === undefined ? "" : deriveInstanceId(driver, label));
  const driverSettingsFields = useMemo(
    () =>
      selectedManifest === undefined && driverOption !== undefined
        ? deriveProviderSettingsFields(driverOption)
        : [],
    [driverOption, selectedManifest],
  );
  const normalizedAdapterSchema = useMemo(
    () =>
      selectedManifest === undefined
        ? undefined
        : normalizeAdapterConfigSchema(selectedManifest.configSchema),
    [selectedManifest],
  );
  const defaultConfig =
    normalizedAdapterSchema === undefined
      ? undefined
      : defaultAdapterConfig(normalizedAdapterSchema);
  const configDraft = configBySelection[selectionKey] ?? defaultConfig;
  const configErrors =
    normalizedAdapterSchema === undefined
      ? {}
      : validateAdapterConfig(normalizedAdapterSchema, configDraft);
  const instanceIdError = validateInstanceId(instanceId, existingIds);
  const showInstanceIdError = hasAttemptedSubmit && instanceIdError !== null;
  const previewLabel = label.trim() || `${selectedLabel} Workspace`;
  const wizardStepSummaries = [selectedLabel, previewLabel, null] as const;

  const setConfigDraft = (config: Record<string, unknown> | undefined) => {
    setConfigBySelection((existing) => {
      const next = { ...existing };
      if (config === undefined || Object.keys(config).length === 0) delete next[selectionKey];
      else next[selectionKey] = config;
      return next;
    });
  };

  const applyWizardNavigation = (navigation: WizardNavigation) => {
    if (navigation.kind === "blocked") {
      setHasAttemptedSubmit(true);
    }
    setWizardStep(navigation.step);
  };

  const navigateToStep = (requestedStep: number) => {
    applyWizardNavigation(
      resolveWizardNavigation(wizardStep, requestedStep, ADD_PROVIDER_WIZARD_STEPS.length, {
        instanceIdError,
      }),
    );
  };

  const handleSave = () => {
    setHasAttemptedSubmit(true);
    if (
      selectionUnavailable ||
      driver === undefined ||
      instanceIdError !== null ||
      Object.keys(configErrors).length > 0
    ) {
      return;
    }

    const config = configDraft;
    const hasConfig = config !== undefined;
    const normalizedAccentColor = normalizeProviderAccentColor(accentColor);

    const nextInstance: ProviderInstanceConfig = buildProviderInstanceConfig({
      driver,
      ...(selectedManifest === undefined ? {} : { manifest: selectedManifest }),
      ...(label.trim().length > 0 ? { displayName: label.trim() } : {}),
      ...(normalizedAccentColor ? { accentColor: normalizedAccentColor } : {}),
      ...(hasConfig ? { config } : {}),
    });
    // `ProviderInstanceId.make` revalidates the slug; we've already checked
    // it via `validateInstanceId`, but going through the brand constructor
    // keeps the type boundary honest and guards against any future drift in
    // the slug rules.
    const brandedId = ProviderInstanceId.make(instanceId);
    const nextMap = {
      ...settings.providerInstances,
      [brandedId]: nextInstance,
    };
    try {
      updateSettings({ providerInstances: nextMap });
      toastManager.add({
        type: "success",
        title: "Provider instance added",
        description: `${selectedLabel} instance '${instanceId}' was added.`,
      });
      onOpenChange(false);
    } catch (error) {
      toastManager.add({
        type: "error",
        title: "Could not add provider instance",
        description: error instanceof Error ? error.message : "Update failed.",
      });
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <WizardPopup>
        <WizardHeader
          title="Add provider instance"
          description={
            <>
              Configure an additional provider instance on {environmentLabel} — for example, a
              second Codex install pointed at a different workspace.
            </>
          }
        >
          <AddProviderInstanceWizardSteps
            currentStep={wizardStep}
            summaries={wizardStepSummaries}
            instanceIdError={instanceIdError}
            onNavigation={applyWizardNavigation}
          />
        </WizardHeader>

        <WizardPanel>
          <div className={cn("grid gap-2", wizardStep !== 0 && "hidden")}>
            <div id="add-instance-driver-label" className="text-sm font-medium text-foreground">
              Driver
            </div>
            <RadioGroup
              value={selectionKey}
              onValueChange={setSelectionKey}
              aria-labelledby="add-instance-driver-label"
              className="grid grid-cols-1 sm:grid-cols-2"
            >
              {DRIVER_OPTIONS.map((option) => {
                const IconComponent = option.icon;
                return (
                  <RadioPrimitive.Root
                    key={option.value}
                    value={selectionKeyForBuiltInDriver(option.value, adapterManifests)}
                    className="relative flex cursor-pointer items-center gap-3 rounded-lg bg-card px-3 py-3 text-left text-muted-foreground outline-none ring-1 ring-black/5 hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring data-checked:bg-primary/8 data-checked:text-foreground data-checked:ring-2 data-checked:ring-primary data-checked:hover:bg-primary/8 dark:bg-white/3 dark:ring-white/5 dark:hover:bg-white/5 dark:data-checked:bg-primary/15 dark:data-checked:ring-primary dark:data-checked:hover:bg-primary/15"
                  >
                    <IconComponent className="size-4 shrink-0" aria-hidden />
                    <span className="min-w-0 flex-1 truncate text-sm font-medium text-foreground">
                      {option.label}
                    </span>
                    <RadioPrimitive.Indicator
                      className="grid size-5 shrink-0 place-items-center rounded-full bg-primary text-primary-foreground"
                      aria-hidden
                    >
                      <CheckIcon className="size-3.5 shrink-0" />
                    </RadioPrimitive.Indicator>
                    {option.badgeLabel ? (
                      <Badge variant="warning" size="sm">
                        {option.badgeLabel}
                      </Badge>
                    ) : null}
                  </RadioPrimitive.Root>
                );
              })}
              {installedAdapterManifests.map((manifest) => {
                const value = adapterManifestSelectionKey(manifest);
                return (
                  <RadioPrimitive.Root
                    key={value}
                    value={value}
                    className="relative flex cursor-pointer items-center gap-3 rounded-lg bg-card px-3 py-3 text-left text-muted-foreground outline-none ring-1 ring-black/5 hover:bg-zinc-50 focus-visible:ring-2 focus-visible:ring-ring data-checked:bg-primary/8 data-checked:text-foreground data-checked:ring-2 data-checked:ring-primary data-checked:hover:bg-primary/8 dark:bg-white/3 dark:ring-white/5 dark:hover:bg-white/5 dark:data-checked:bg-primary/15 dark:data-checked:ring-primary dark:data-checked:hover:bg-primary/15"
                  >
                    <PackageIcon className="size-4 shrink-0" aria-hidden />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-sm font-medium text-foreground">
                        {manifest.displayName}
                      </span>
                      <span className="block truncate text-[10px] text-muted-foreground">
                        {manifest.driver} · {manifest.id}@{manifest.version}
                      </span>
                    </span>
                    <RadioPrimitive.Indicator
                      className="grid size-5 shrink-0 place-items-center rounded-full bg-primary text-primary-foreground"
                      aria-hidden
                    >
                      <CheckIcon className="size-3.5 shrink-0" />
                    </RadioPrimitive.Indicator>
                  </RadioPrimitive.Root>
                );
              })}
              {COMING_SOON_DRIVER_OPTIONS.map((option) => {
                const IconComponent = option.icon;
                return (
                  <RadioPrimitive.Root
                    key={option.value}
                    value={`coming-soon:${option.value}`}
                    disabled
                    className={cn(
                      "relative flex cursor-not-allowed items-center gap-3 rounded-lg bg-card/60 px-3 py-3 text-left opacity-64 outline-none ring-1 ring-black/5 dark:bg-white/2 dark:ring-white/5",
                    )}
                  >
                    <IconComponent className="size-4 shrink-0 text-muted-foreground" aria-hidden />
                    <span className="min-w-0 flex-1 truncate text-sm font-medium text-foreground">
                      {option.label}
                    </span>
                    <Badge variant="warning" size="sm">
                      Coming Soon
                    </Badge>
                  </RadioPrimitive.Root>
                );
              })}
            </RadioGroup>
          </div>

          <label className={cn("grid gap-2", wizardStep !== 1 && "hidden")}>
            <span className="text-xs font-medium text-foreground">Label</span>
            <Input
              placeholder="e.g. Work"
              value={label}
              onChange={(event) => setLabel(event.target.value)}
            />
            <span className="text-2xs text-muted-foreground">
              Shown in the provider list. Optional.
            </span>
          </label>

          <label className={cn("grid gap-2", wizardStep !== 1 && "hidden")}>
            <span className="text-xs font-medium text-foreground">Instance ID</span>
            <Input
              className="bg-background"
              placeholder={`${driver ?? "adapter"}_work`}
              value={instanceId}
              onChange={(event) => {
                setInstanceIdOverride(event.target.value);
              }}
              aria-invalid={showInstanceIdError}
            />
            {showInstanceIdError ? (
              <span className="text-2xs text-destructive">{instanceIdError}</span>
            ) : (
              <span className="text-2xs text-muted-foreground">
                Routing key used by threads and sessions. Letters, digits, '-', or '_'.
              </span>
            )}
          </label>

          <div className={cn("grid gap-2", wizardStep !== 1 && "hidden")}>
            <span className="text-xs font-medium text-foreground">Accent color</span>
            <div className="flex min-w-0 flex-wrap items-center gap-2">
              <ProviderAccentColorPicker
                displayName={label || driverOption.label}
                value={accentColor || undefined}
                onCommit={setAccentColor}
                layout="inline"
              />
              <div className="flex flex-wrap gap-1.5">
                {PROVIDER_ACCENT_SWATCHES.map((swatch) => {
                  const selected = accentColor.toLowerCase() === swatch;
                  return (
                    <button
                      key={swatch}
                      type="button"
                      className={cn(
                        "size-6 cursor-pointer rounded-full border transition",
                        selected
                          ? "scale-110 border-foreground ring-2 ring-ring ring-offset-1 ring-offset-background"
                          : "border-black/10 hover:scale-105 dark:border-white/20",
                      )}
                      style={{ backgroundColor: swatch }}
                      onClick={() => setAccentColor(swatch)}
                      aria-label={`Use ${swatch} accent`}
                    />
                  );
                })}
              </div>
              {accentColor ? (
                <Button
                  type="button"
                  size="xs"
                  variant="ghost-muted"
                  onClick={() => setAccentColor("")}
                >
                  Clear
                </Button>
              ) : null}
            </div>
            <span className="text-2xs text-muted-foreground">
              Optional marker shown in the picker.
            </span>
          </div>

          {selectedManifest !== undefined ? (
            <div className={cn("grid gap-4", wizardStep !== 2 && "hidden")}>
              <AdapterConfigForm
                schema={selectedManifest.configSchema}
                value={configDraft}
                idPrefix={`add-provider-${selectionKey}`}
                variant="dialog"
                showErrors={hasAttemptedSubmit}
                onChange={setConfigDraft}
              />
              {hasAttemptedSubmit && Object.keys(configErrors).length > 0 ? (
                <p className="text-xs text-destructive">
                  Fix the highlighted adapter configuration before adding this instance.
                </p>
              ) : null}
            </div>
          ) : driverOption !== undefined && driverSettingsFields.length > 0 ? (
            <div className={cn("grid gap-4", wizardStep !== 2 && "hidden")}>
              <ProviderSettingsForm
                definition={driverOption}
                value={configDraft}
                idPrefix={`add-provider-${selectionKey}`}
                variant="dialog"
                onChange={setConfigDraft}
              />
            </div>
          ) : wizardStep === 2 ? (
            <div className="grid gap-2">
              <p className="text-sm text-muted-foreground">
                This driver has no required configuration. You can add the instance now.
              </p>
            </div>
          ) : null}
          {selectionUnavailable ? (
            <p className="text-xs text-destructive" role="alert">
              This adapter package is no longer installed. Choose an available provider before
              adding the instance.
            </p>
          ) : null}
        </WizardPanel>

        <WizardFooter>
          <Button
            variant="outline"
            onClick={() => {
              if (wizardStep === 0) {
                onOpenChange(false);
                return;
              }
              setWizardStep((step) => Math.max(0, step - 1));
            }}
          >
            {wizardStep === 0 ? "Cancel" : "Back"}
          </Button>
          {wizardStep < ADD_PROVIDER_WIZARD_STEPS.length - 1 ? (
            <Button onClick={() => navigateToStep(wizardStep + 1)}>Next</Button>
          ) : (
            <Button onClick={handleSave}>Add instance</Button>
          )}
        </WizardFooter>
      </WizardPopup>
    </Dialog>
  );
}
