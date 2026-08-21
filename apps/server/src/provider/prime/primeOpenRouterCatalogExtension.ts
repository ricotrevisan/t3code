import * as Effect from "effect/Effect";
import * as FileSystem from "effect/FileSystem";
import * as Path from "effect/Path";

const PRIME_OPENROUTER_CATALOG_EXTENSION_FILE_NAME = "t3-openrouter-catalog.ts";

const PRIME_OPENROUTER_CATALOG_EXTENSION_SOURCE = `import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

type UnknownRecord = Record<string, unknown>;

const isRecord = (value: unknown): value is UnknownRecord =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const finitePositiveInteger = (value: unknown, fallback: number): number =>
  typeof value === "number" && Number.isFinite(value) && value > 0
    ? Math.floor(value)
    : fallback;

const costPerMillion = (value: unknown): number => {
  const parsed = typeof value === "string" || typeof value === "number" ? Number(value) : 0;
  return Number.isFinite(parsed) && parsed >= 0 ? parsed * 1_000_000 : 0;
};

const thinkingLevelMap = (reasoning: UnknownRecord | undefined) => {
  const efforts = Array.isArray(reasoning?.supported_efforts)
    ? reasoning.supported_efforts.filter((value): value is string => typeof value === "string")
    : [];
  if (efforts.length === 0) {
    return undefined;
  }
  const supported = new Set(efforts);
  const map: Record<string, string | null> = {
    off: reasoning?.mandatory === true ? null : supported.has("none") ? "none" : null,
    minimal: supported.has("minimal") ? "minimal" : null,
    low: supported.has("low") ? "low" : null,
    medium: supported.has("medium") ? "medium" : null,
    high: supported.has("high") ? "high" : null,
    xhigh: supported.has("xhigh") ? "xhigh" : null,
    max: supported.has("max") ? "max" : null,
  };
  return map;
};

const toPrimeModel = (value: unknown) => {
  if (!isRecord(value) || typeof value.id !== "string" || value.id.trim().length === 0) {
    return undefined;
  }
  const id = value.id.trim();
  const architecture = isRecord(value.architecture) ? value.architecture : undefined;
  const inputModalities = Array.isArray(architecture?.input_modalities)
    ? architecture.input_modalities
    : [];
  const input: Array<"text" | "image"> = ["text"];
  if (inputModalities.includes("image")) {
    input.push("image");
  }
  const pricing = isRecord(value.pricing) ? value.pricing : undefined;
  const topProvider = isRecord(value.top_provider) ? value.top_provider : undefined;
  const reasoning = isRecord(value.reasoning) ? value.reasoning : undefined;
  const supportedParameters = Array.isArray(value.supported_parameters)
    ? value.supported_parameters
    : [];
  const supportsReasoning =
    reasoning !== undefined ||
    supportedParameters.includes("reasoning") ||
    supportedParameters.includes("include_reasoning");
  const levelMap = thinkingLevelMap(reasoning);
  return {
    id,
    name: typeof value.name === "string" && value.name.trim().length > 0 ? value.name.trim() : id,
    reasoning: supportsReasoning,
    ...(levelMap === undefined ? {} : { thinkingLevelMap: levelMap }),
    input,
    cost: {
      input: costPerMillion(pricing?.prompt),
      output: costPerMillion(pricing?.completion),
      cacheRead: costPerMillion(pricing?.input_cache_read),
      cacheWrite: costPerMillion(pricing?.input_cache_write),
    },
    contextWindow: finitePositiveInteger(value.context_length, 128_000),
    maxTokens: finitePositiveInteger(topProvider?.max_completion_tokens, 16_384),
  };
};

export default async function t3OpenRouterCatalogExtension(pi: ExtensionAPI) {
  try {
    const response = await fetch("https://openrouter.ai/api/v1/models", {
      headers: { accept: "application/json" },
      signal: AbortSignal.timeout(5_000),
    });
    if (!response.ok) {
      return;
    }
    const payload: unknown = await response.json();
    if (!isRecord(payload) || !Array.isArray(payload.data)) {
      return;
    }
    const models = payload.data.map(toPrimeModel).filter((model) => model !== undefined);
    if (models.length === 0) {
      return;
    }
    pi.registerProvider("openrouter", {
      name: "OpenRouter",
      baseUrl: "https://openrouter.ai/api/v1",
      apiKey: "OPENROUTER_API_KEY",
      api: "openai-completions",
      models,
    });
  } catch {
    // Keep Prime's bundled catalog when OpenRouter is unavailable.
  }
}
`;

export const preparePrimeOpenRouterCatalogExtension = Effect.fn(
  "preparePrimeOpenRouterCatalogExtension",
)(function* (baseDir: string) {
  const fileSystem = yield* FileSystem.FileSystem;
  const path = yield* Path.Path;
  const extensionDirectory = path.resolve(baseDir, "prime-agent", "extensions");
  const extensionPath = path.join(extensionDirectory, PRIME_OPENROUTER_CATALOG_EXTENSION_FILE_NAME);

  yield* fileSystem.makeDirectory(extensionDirectory, { recursive: true });
  yield* fileSystem.writeFileString(extensionPath, PRIME_OPENROUTER_CATALOG_EXTENSION_SOURCE);

  return extensionPath;
});
