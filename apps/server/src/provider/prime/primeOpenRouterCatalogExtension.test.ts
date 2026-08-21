// @effect-diagnostics nodeBuiltinImport:off
import * as NodeURL from "node:url";

import * as NodeServices from "@effect/platform-node/NodeServices";
import { it } from "@effect/vitest";
import * as Effect from "effect/Effect";
import * as FileSystem from "effect/FileSystem";
import { describe, expect } from "vite-plus/test";

import { preparePrimeOpenRouterCatalogExtension } from "./primeOpenRouterCatalogExtension.ts";

const provide = <A, E, R>(effect: Effect.Effect<A, E, R>) =>
  effect.pipe(Effect.provide(NodeServices.layer));

interface ProviderRegistration {
  readonly name: string;
  readonly config: Record<string, unknown>;
}

const loadExtension = (
  extensionPath: string,
  response: { readonly ok: boolean; readonly json: () => Promise<unknown> },
) =>
  Effect.gen(function* () {
    const registrations: Array<ProviderRegistration> = [];
    const loaded = yield* Effect.tryPromise(
      () => import(NodeURL.pathToFileURL(extensionPath).href),
    );
    const factory = loaded.default as (api: {
      registerProvider(name: string, config: Record<string, unknown>): void;
    }) => Promise<void>;
    const originalFetch = globalThis.fetch;
    globalThis.fetch = (async (input: string | URL | Request) => {
      expect(String(input)).toBe("https://openrouter.ai/api/v1/models");
      return response as Response;
    }) as typeof fetch;
    try {
      yield* Effect.promise(() =>
        factory({
          registerProvider: (name, config) => registrations.push({ name, config }),
        }),
      );
    } finally {
      globalThis.fetch = originalFetch;
    }
    return registrations;
  });

describe("primeOpenRouterCatalogExtension", () => {
  it.live("adds the current OpenRouter catalog with Ox Alpha metadata", () =>
    provide(
      Effect.scoped(
        Effect.gen(function* () {
          const fileSystem = yield* FileSystem.FileSystem;
          const baseDir = yield* fileSystem.makeTempDirectoryScoped({
            prefix: "t3-prime-openrouter-catalog-",
          });
          const extensionPath = yield* preparePrimeOpenRouterCatalogExtension(baseDir);
          const registrations = yield* loadExtension(extensionPath, {
            ok: true,
            json: async () => ({
              data: [
                {
                  id: "stealth/ox-alpha",
                  name: "Ox Alpha",
                  context_length: 1_048_576,
                  architecture: { input_modalities: ["text", "image", "video"] },
                  pricing: { prompt: "0", completion: "0" },
                  top_provider: { max_completion_tokens: 131_072 },
                  supported_parameters: ["reasoning", "reasoning_effort", "tools"],
                  reasoning: {
                    mandatory: true,
                    supported_efforts: ["max", "high", "low"],
                  },
                },
              ],
            }),
          });

          expect(registrations).toHaveLength(1);
          expect(registrations[0]).toEqual({
            name: "openrouter",
            config: {
              name: "OpenRouter",
              baseUrl: "https://openrouter.ai/api/v1",
              apiKey: "OPENROUTER_API_KEY",
              api: "openai-completions",
              models: [
                {
                  id: "stealth/ox-alpha",
                  name: "Ox Alpha",
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
                  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
                  contextWindow: 1_048_576,
                  maxTokens: 131_072,
                },
              ],
            },
          });
        }),
      ),
    ),
  );

  it.live("keeps Prime's bundled OpenRouter catalog when refresh fails", () =>
    provide(
      Effect.scoped(
        Effect.gen(function* () {
          const fileSystem = yield* FileSystem.FileSystem;
          const baseDir = yield* fileSystem.makeTempDirectoryScoped({
            prefix: "t3-prime-openrouter-catalog-failure-",
          });
          const extensionPath = yield* preparePrimeOpenRouterCatalogExtension(baseDir);
          const registrations = yield* loadExtension(extensionPath, {
            ok: false,
            json: async () => ({ error: "unavailable" }),
          });

          expect(registrations).toEqual([]);
        }),
      ),
    ),
  );
});
