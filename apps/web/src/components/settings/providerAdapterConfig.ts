import type {
  ProviderAdapterConfigSchema,
  ProviderAdapterManifestV1,
  ProviderAdapterPackageReference,
  ProviderInstanceConfig,
  ProviderDriverKind,
  ServerProvider,
} from "@t3tools/contracts";

export type AdapterConfigFieldKind =
  | "string"
  | "boolean"
  | "number"
  | "integer"
  | "string-enum"
  | "string-array"
  | "unsupported";

export type AdapterConfigDefault = string | boolean | number | ReadonlyArray<string>;

export interface AdapterConfigFieldModel {
  readonly key: string;
  readonly kind: AdapterConfigFieldKind;
  readonly label: string;
  readonly description?: string;
  readonly required: boolean;
  readonly defaultValue?: unknown;
  readonly hasDefault: boolean;
  readonly options?: ReadonlyArray<string>;
  readonly unsupportedReason?: string;
}

export interface NormalizedAdapterConfigSchema {
  readonly fields: ReadonlyArray<AdapterConfigFieldModel>;
  readonly unsupportedRoot: boolean;
  readonly hasRootDefault: boolean;
  readonly rootDefault?: unknown;
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === "object" && !Array.isArray(value);

const titleize = (key: string): string =>
  key
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/[-_]+/g, " ")
    .replace(/^./, (character) => character.toUpperCase());

const readString = (value: unknown): string | undefined =>
  typeof value === "string" && value.trim().length > 0 ? value : undefined;

const supportedDefault = (
  kind: Exclude<AdapterConfigFieldKind, "unsupported">,
  value: unknown,
  options?: ReadonlyArray<string>,
): AdapterConfigDefault | undefined => {
  if (kind === "string") return typeof value === "string" ? value : undefined;
  if (kind === "boolean") return typeof value === "boolean" ? value : undefined;
  if (kind === "number")
    return typeof value === "number" && Number.isFinite(value) ? value : undefined;
  if (kind === "integer") {
    return typeof value === "number" && Number.isInteger(value) ? value : undefined;
  }
  if (kind === "string-enum") {
    return typeof value === "string" && options?.includes(value) ? value : undefined;
  }
  return Array.isArray(value) && value.every((entry) => typeof entry === "string")
    ? [...value]
    : undefined;
};

/** Normalize the deliberately small JSON-Schema-shaped subset rendered by the web client. */
export function normalizeAdapterConfigSchema(
  schema: ProviderAdapterConfigSchema,
): NormalizedAdapterConfigSchema {
  const root = schema as Record<string, unknown>;
  const properties = root.properties === undefined ? {} : root.properties;
  if (root.type !== "object" || !isRecord(properties)) {
    const hasRootDefault = Object.hasOwn(root, "default");
    return {
      fields: [],
      unsupportedRoot: true,
      hasRootDefault,
      ...(hasRootDefault ? { rootDefault: root.default } : {}),
    };
  }

  const required = new Set(
    Array.isArray(root.required)
      ? root.required.filter((value): value is string => typeof value === "string")
      : [],
  );
  const fields = Object.entries(properties).map(([key, rawField]) => {
    const field = isRecord(rawField) ? rawField : {};
    const description = readString(field.description);
    const hasDeclaredDefault = Object.hasOwn(field, "default");
    const base = {
      key,
      label: readString(field.title) ?? titleize(key),
      ...(description === undefined ? {} : { description }),
      required: required.has(key),
    };

    let kind: Exclude<AdapterConfigFieldKind, "unsupported"> | undefined;
    let options: ReadonlyArray<string> | undefined;
    if (
      field.type === "string" &&
      Array.isArray(field.enum) &&
      field.enum.length > 0 &&
      field.enum.every((value) => typeof value === "string")
    ) {
      kind = "string-enum";
      options = field.enum as ReadonlyArray<string>;
    } else if (field.type === "string" && field.enum === undefined) {
      // `format: "password"` is intentionally not special. Secrets belong in
      // the existing per-instance environment-variable editor.
      kind = "string";
    } else if (field.type === "boolean") {
      kind = "boolean";
    } else if (field.type === "number") {
      kind = "number";
    } else if (field.type === "integer") {
      kind = "integer";
    } else if (field.type === "array" && isRecord(field.items) && field.items.type === "string") {
      kind = "string-array";
    }

    if (kind === undefined) {
      return {
        ...base,
        kind: "unsupported",
        hasDefault: hasDeclaredDefault,
        ...(hasDeclaredDefault ? { defaultValue: field.default } : {}),
        unsupportedReason:
          "This field uses a schema shape that this client cannot edit. Its value is preserved.",
      } satisfies AdapterConfigFieldModel;
    }

    const defaultValue = supportedDefault(kind, field.default, options);
    return {
      ...base,
      kind,
      hasDefault: defaultValue !== undefined,
      ...(options === undefined ? {} : { options }),
      ...(defaultValue === undefined ? {} : { defaultValue }),
    } satisfies AdapterConfigFieldModel;
  });

  return { fields, unsupportedRoot: false, hasRootDefault: false };
}

export function defaultAdapterConfig(normalized: NormalizedAdapterConfigSchema): unknown {
  if (normalized.unsupportedRoot) {
    return normalized.hasRootDefault ? normalized.rootDefault : undefined;
  }
  const config: Record<string, unknown> = {};
  for (const field of normalized.fields) {
    if (field.hasDefault) {
      config[field.key] = Array.isArray(field.defaultValue)
        ? [...field.defaultValue]
        : field.defaultValue;
    }
  }
  return Object.keys(config).length > 0 ? config : undefined;
}

export function nextAdapterConfigWithFieldValue(
  config: unknown,
  field: AdapterConfigFieldModel,
  value: AdapterConfigDefault | undefined,
): Record<string, unknown> | undefined {
  const next = isRecord(config) ? { ...config } : {};
  if (value === undefined) delete next[field.key];
  else next[field.key] = Array.isArray(value) ? [...value] : value;
  return Object.keys(next).length > 0 ? next : undefined;
}

export function validateAdapterConfig(
  normalized: NormalizedAdapterConfigSchema,
  config: unknown,
): Readonly<Record<string, string>> {
  if (normalized.unsupportedRoot) {
    return config === undefined
      ? { $root: "This adapter's required configuration cannot be edited by this client." }
      : {};
  }
  const record = isRecord(config) ? config : {};
  const errors: Record<string, string> = {};
  for (const field of normalized.fields) {
    const value = record[field.key];
    if (field.kind === "unsupported") {
      if (field.required && value === undefined) {
        errors[field.key] = `${field.label} is required but cannot be edited by this client.`;
      }
      continue;
    }
    if (value === undefined) {
      if (field.required) errors[field.key] = `${field.label} is required.`;
      continue;
    }
    const invalid =
      (field.kind === "string" && typeof value !== "string") ||
      (field.kind === "boolean" && typeof value !== "boolean") ||
      (field.kind === "number" && (typeof value !== "number" || !Number.isFinite(value))) ||
      (field.kind === "integer" && (typeof value !== "number" || !Number.isInteger(value))) ||
      (field.kind === "string-enum" &&
        (typeof value !== "string" || !field.options?.includes(value))) ||
      (field.kind === "string-array" &&
        (!Array.isArray(value) || !value.every((entry) => typeof entry === "string")));
    if (invalid) errors[field.key] = `${field.label} has an invalid value.`;
  }
  return errors;
}

export function sameAdapterPackage(
  left: ProviderAdapterPackageReference | undefined,
  right: ProviderAdapterPackageReference | undefined,
): boolean {
  return (
    left !== undefined &&
    right !== undefined &&
    left.id === right.id &&
    left.version === right.version &&
    left.protocolVersion === right.protocolVersion
  );
}

export const adapterPackageIdentity = (reference: ProviderAdapterPackageReference): string =>
  `${reference.id}:${reference.version}:${reference.protocolVersion}`;

export function manifestPackageReference(
  manifest: ProviderAdapterManifestV1,
): ProviderAdapterPackageReference {
  return {
    id: manifest.id,
    version: manifest.version,
    protocolVersion: manifest.protocolVersion,
  };
}

export function buildProviderInstanceConfig(input: {
  readonly driver: ProviderInstanceConfig["driver"];
  readonly manifest?: ProviderAdapterManifestV1;
  readonly displayName?: string;
  readonly accentColor?: string;
  readonly config?: unknown;
}): ProviderInstanceConfig {
  const hasConfig =
    input.config !== undefined && (!isRecord(input.config) || Object.keys(input.config).length > 0);
  return {
    driver: input.driver,
    ...(input.manifest === undefined
      ? {}
      : { adapterPackage: manifestPackageReference(input.manifest) }),
    enabled: true,
    ...(input.displayName === undefined ? {} : { displayName: input.displayName }),
    ...(input.accentColor === undefined ? {} : { accentColor: input.accentColor }),
    ...(hasConfig ? { config: input.config } : {}),
  };
}

export const adapterManifestSelectionKey = (manifest: ProviderAdapterManifestV1): string =>
  `adapter:${manifest.id}:${manifest.version}:${manifest.protocolVersion}`;

export function selectionKeyForBuiltInDriver(
  driver: ProviderDriverKind,
  manifests: ReadonlyArray<ProviderAdapterManifestV1>,
): string {
  const manifest = manifests.find((candidate) => candidate.driver === driver);
  return manifest === undefined ? `built-in:${driver}` : adapterManifestSelectionKey(manifest);
}

export type AdapterSelectionResolution =
  | {
      readonly kind: "built-in";
      readonly driver: ProviderDriverKind;
      readonly manifest?: ProviderAdapterManifestV1;
    }
  | { readonly kind: "manifest"; readonly manifest: ProviderAdapterManifestV1 }
  | { readonly kind: "missing" };

/** Resolve a stored wizard choice without ever falling back to another provider. */
export function resolveAdapterSelection(
  selectionKey: string,
  builtInDrivers: ReadonlyArray<ProviderDriverKind>,
  manifests: ReadonlyArray<ProviderAdapterManifestV1>,
): AdapterSelectionResolution {
  if (selectionKey.startsWith("built-in:")) {
    const requested = selectionKey.slice("built-in:".length);
    const driver = builtInDrivers.find((candidate) => candidate === requested);
    if (driver === undefined) return { kind: "missing" };
    const manifest = manifests.find((candidate) => candidate.driver === driver);
    return {
      kind: "built-in",
      driver,
      ...(manifest === undefined ? {} : { manifest }),
    };
  }
  const manifest = manifests.find(
    (candidate) => adapterManifestSelectionKey(candidate) === selectionKey,
  );
  return manifest === undefined ? { kind: "missing" } : { kind: "manifest", manifest };
}

/** Hide catalog entries already represented by a compiled built-in driver choice. */
export function addableAdapterManifests(
  manifests: ReadonlyArray<ProviderAdapterManifestV1>,
  builtInDrivers: ReadonlySet<string>,
): ReadonlyArray<ProviderAdapterManifestV1> {
  const seen = new Set<string>();
  return manifests.filter((manifest) => {
    if (builtInDrivers.has(String(manifest.driver))) return false;
    const identity = `${manifest.id}\0${manifest.version}\0${manifest.protocolVersion}`;
    if (seen.has(identity)) return false;
    seen.add(identity);
    return true;
  });
}

/** Resolve only a schema whose package identity and driver exactly match the configured instance. */
export function resolveAdapterConfigSchema(input: {
  readonly instance: ProviderInstanceConfig;
  readonly liveProvider: ServerProvider | undefined;
  readonly manifests: ReadonlyArray<ProviderAdapterManifestV1>;
}): ProviderAdapterConfigSchema | undefined {
  const packageReference = input.instance.adapterPackage;
  if (packageReference === undefined) return undefined;

  if (
    input.liveProvider?.adapterConfigSchema !== undefined &&
    input.liveProvider.driver === input.instance.driver &&
    sameAdapterPackage(input.liveProvider.adapterPackage, packageReference)
  ) {
    return input.liveProvider.adapterConfigSchema;
  }

  return input.manifests.find(
    (manifest) =>
      manifest.driver === input.instance.driver &&
      sameAdapterPackage(manifestPackageReference(manifest), packageReference),
  )?.configSchema;
}
