import * as NodeServices from "@effect/platform-node/NodeServices";
import { describe, expect, it } from "@effect/vitest";
import * as Effect from "effect/Effect";
import * as FileSystem from "effect/FileSystem";
import * as Path from "effect/Path";

import {
  npmPackageNameFromSpec,
  primePackageCatalogExtensionArgs,
  resolvePrimeAgentDir,
  resolvePrimePackageCatalogExtensionPaths,
} from "./primePackageCatalogExtensions.ts";

const provide = <A, E, R>(effect: Effect.Effect<A, E, R>) =>
  effect.pipe(Effect.provide(NodeServices.layer));

describe("npmPackageNameFromSpec", () => {
  it("parses npm package specs and ignores other sources", () => {
    expect(npmPackageNameFromSpec("npm:pi-xai-oauth")).toBe("pi-xai-oauth");
    expect(npmPackageNameFromSpec("npm:pi-xai-oauth@1.5.2")).toBe("pi-xai-oauth");
    expect(npmPackageNameFromSpec("npm:@blockedpath/pi-xai-oauth")).toBe(
      "@blockedpath/pi-xai-oauth",
    );
    expect(npmPackageNameFromSpec("npm:@blockedpath/pi-xai-oauth@1.5.2")).toBe(
      "@blockedpath/pi-xai-oauth",
    );
    expect(npmPackageNameFromSpec("git:https://example.com/pkg.git")).toBeUndefined();
    expect(npmPackageNameFromSpec("")).toBeUndefined();
  });
});

describe("resolvePrimeAgentDir", () => {
  it("prefers PRIME_AGENT_CODING_AGENT_DIR over the home default", () => {
    expect(resolvePrimeAgentDir({ PRIME_AGENT_CODING_AGENT_DIR: "/tmp/prime-agent-dir" })).toBe(
      "/tmp/prime-agent-dir",
    );
  });
});

describe("resolvePrimePackageCatalogExtensionPaths", () => {
  it.effect("expands installed npm package catalogs to loadable extension files", () =>
    provide(
      Effect.scoped(
        Effect.gen(function* () {
          const fileSystem = yield* FileSystem.FileSystem;
          const path = yield* Path.Path;
          const agentDir = yield* fileSystem.makeTempDirectoryScoped({
            prefix: "t3code-prime-package-catalog-",
          });
          const xaiRoot = path.join(agentDir, "npm", "node_modules", "pi-xai-oauth");
          const xaiExtensionDir = path.join(xaiRoot, "extensions");
          const clineRoot = path.join(agentDir, "npm", "node_modules", "pi-clinepass-provider");
          const clineExtension = path.join(clineRoot, "src", "index.ts");
          const conventionRoot = path.join(agentDir, "npm", "node_modules", "pi-convention");
          const conventionExtension = path.join(conventionRoot, "extensions", "index.ts");
          yield* fileSystem.makeDirectory(path.join(xaiExtensionDir, "xai"), { recursive: true });
          yield* fileSystem.makeDirectory(path.join(clineRoot, "src"), { recursive: true });
          yield* fileSystem.makeDirectory(path.dirname(conventionExtension), { recursive: true });
          yield* fileSystem.writeFileString(
            path.join(xaiRoot, "package.json"),
            // @effect-diagnostics-next-line preferSchemaOverJson:off - fixed package manifest fixture.
            JSON.stringify({
              name: "pi-xai-oauth",
              pi: { extensions: ["./extensions", "!extensions/disabled.ts"] },
            }),
          );
          yield* fileSystem.writeFileString(
            path.join(xaiExtensionDir, "xai-oauth.ts"),
            "export default async function () {}",
          );
          yield* fileSystem.writeFileString(
            path.join(xaiExtensionDir, "disabled.ts"),
            "export default async function () {}",
          );
          yield* fileSystem.writeFileString(
            path.join(xaiExtensionDir, "xai", "catalog.ts"),
            "export const ignored = true;",
          );
          yield* fileSystem.writeFileString(
            path.join(clineRoot, "package.json"),
            // @effect-diagnostics-next-line preferSchemaOverJson:off - fixed package manifest fixture.
            JSON.stringify({
              name: "pi-clinepass-provider",
              pi: { extensions: ["./src/*.ts"] },
            }),
          );
          yield* fileSystem.writeFileString(clineExtension, "export default async function () {}");
          yield* fileSystem.writeFileString(
            path.join(conventionRoot, "package.json"),
            // @effect-diagnostics-next-line preferSchemaOverJson:off - fixed package manifest fixture.
            JSON.stringify({ name: "pi-convention" }),
          );
          yield* fileSystem.writeFileString(
            conventionExtension,
            "export default async function () {}",
          );
          yield* fileSystem.writeFileString(
            path.join(agentDir, "settings.json"),
            // @effect-diagnostics-next-line preferSchemaOverJson:off - fixed Prime settings fixture.
            JSON.stringify({
              packages: [
                "npm:pi-clinepass-provider",
                { source: "npm:pi-xai-oauth@1.5.2" },
                "npm:pi-convention",
                "git:https://example.com/pkg.git",
              ],
            }),
          );

          expect(resolvePrimePackageCatalogExtensionPaths(agentDir)).toEqual([
            clineExtension,
            path.join(xaiExtensionDir, "xai-oauth.ts"),
            conventionExtension,
          ]);
          expect(primePackageCatalogExtensionArgs(agentDir)).toEqual([
            "--extension",
            clineExtension,
            "--extension",
            path.join(xaiExtensionDir, "xai-oauth.ts"),
            "--extension",
            conventionExtension,
          ]);

          yield* fileSystem.writeFileString(
            path.join(agentDir, "settings.json"),
            // @effect-diagnostics-next-line preferSchemaOverJson:off - fixed package filter fixture.
            JSON.stringify({
              packages: [
                { source: "npm:pi-xai-oauth", extensions: [] },
                { source: "npm:pi-clinepass-provider", autoload: false },
              ],
            }),
          );

          expect(resolvePrimePackageCatalogExtensionPaths(agentDir)).toEqual([]);
        }),
      ),
    ),
  );

  it("returns no paths when settings or packages are missing", () => {
    expect(resolvePrimePackageCatalogExtensionPaths("/definitely/not-a-prime-agent-dir")).toEqual(
      [],
    );
  });
});
