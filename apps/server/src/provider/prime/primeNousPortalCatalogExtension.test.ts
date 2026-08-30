import * as NodeURL from "node:url";

import * as NodeServices from "@effect/platform-node/NodeServices";
import { it } from "@effect/vitest";
import * as Effect from "effect/Effect";
import * as FileSystem from "effect/FileSystem";
import { describe, expect } from "vite-plus/test";

import { preparePrimeNousPortalCatalogExtension } from "./primeNousPortalCatalogExtension.ts";

const provide = <A, E, R>(effect: Effect.Effect<A, E, R>) =>
  effect.pipe(Effect.provide(NodeServices.layer));

describe("primeNousPortalCatalogExtension", () => {
  it.live("registers only GLM 5.3 Flash on the Nous Portal provider", () =>
    provide(
      Effect.scoped(
        Effect.gen(function* () {
          const fileSystem = yield* FileSystem.FileSystem;
          const baseDir = yield* fileSystem.makeTempDirectoryScoped({
            prefix: "t3-prime-nous-portal-catalog-",
          });
          const extensionPath = yield* preparePrimeNousPortalCatalogExtension(baseDir);
          const loaded = yield* Effect.tryPromise(
            () => import(NodeURL.pathToFileURL(extensionPath).href),
          );
          const factory = loaded.default as (api: {
            registerProvider(name: string, config: Record<string, unknown>): void;
          }) => Promise<void>;
          const registrations: Array<{ name: string; config: Record<string, unknown> }> = [];
          yield* Effect.promise(() =>
            factory({
              registerProvider: (name, config) => registrations.push({ name, config }),
            }),
          );

          expect(registrations).toEqual([
            {
              name: "nous-portal-api-key",
              config: {
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
              },
            },
          ]);
        }),
      ),
    ),
  );
});
