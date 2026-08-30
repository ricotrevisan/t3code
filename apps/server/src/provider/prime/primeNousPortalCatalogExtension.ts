import * as Effect from "effect/Effect";
import * as FileSystem from "effect/FileSystem";
import * as Path from "effect/Path";

const PRIME_NOUS_PORTAL_CATALOG_EXTENSION_FILE_NAME = "t3-nous-portal-catalog.ts";

/**
 * Static Nous Research Portal catalog. Inference is OpenAI-compatible
 * (`https://inference-api.nousresearch.com/v1`) with `NOUS_API_KEY`.
 * Only GLM 5.3 Flash is registered on purpose.
 */
const PRIME_NOUS_PORTAL_CATALOG_EXTENSION_SOURCE = `import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export default async function t3NousPortalCatalogExtension(pi: ExtensionAPI) {
  pi.registerProvider("nous-portal-api-key", {
    name: "Nous Research Portal",
    baseUrl: "https://inference-api.nousresearch.com/v1",
    apiKey: "NOUS_API_KEY",
    api: "openai-completions",
    models: [
      {
        id: "z-ai/glm-5.3-flash",
        name: "Z.ai: GLM 5.3 Flash",
        reasoning: true,
        thinkingLevelMap: {
          off: null,
          minimal: null,
          low: "low",
          medium: null,
          high: "high",
          xhigh: null,
          max: "max",
        },
        input: ["text", "image"],
        cost: {
          input: 0.06,
          output: 0.2,
          cacheRead: 0.012,
          cacheWrite: 0,
        },
        contextWindow: 1_310_720,
        maxTokens: 131_072,
      },
    ],
  });
}
`;

export const preparePrimeNousPortalCatalogExtension = Effect.fn(
  "preparePrimeNousPortalCatalogExtension",
)(function* (baseDir: string) {
  const fileSystem = yield* FileSystem.FileSystem;
  const path = yield* Path.Path;
  const extensionDirectory = path.resolve(baseDir, "prime-agent", "extensions");
  const extensionPath = path.join(
    extensionDirectory,
    PRIME_NOUS_PORTAL_CATALOG_EXTENSION_FILE_NAME,
  );

  yield* fileSystem.makeDirectory(extensionDirectory, { recursive: true });
  yield* fileSystem.writeFileString(extensionPath, PRIME_NOUS_PORTAL_CATALOG_EXTENSION_SOURCE);

  return extensionPath;
});
