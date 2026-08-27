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
  it.effect("returns installed npm package extension roots from settings.json", () =>
    provide(
      Effect.scoped(
        Effect.gen(function* () {
          const fileSystem = yield* FileSystem.FileSystem;
          const path = yield* Path.Path;
          const agentDir = yield* fileSystem.makeTempDirectoryScoped({
            prefix: "t3code-prime-package-catalog-",
          });
          const packageRoot = path.join(agentDir, "npm", "node_modules", "pi-xai-oauth");
          const extensionDir = path.join(packageRoot, "extensions");
          yield* fileSystem.makeDirectory(extensionDir, { recursive: true });
          yield* fileSystem.writeFileString(
            path.join(packageRoot, "package.json"),
            JSON.stringify({
              name: "pi-xai-oauth",
              pi: { extensions: ["./extensions"] },
            }),
          );
          yield* fileSystem.writeFileString(
            path.join(agentDir, "settings.json"),
            JSON.stringify({
              packages: ["npm:pi-clinepass-provider", "npm:pi-xai-oauth@1.5.2"],
            }),
          );

          expect(resolvePrimePackageCatalogExtensionPaths(agentDir)).toEqual([extensionDir]);
          expect(primePackageCatalogExtensionArgs(agentDir)).toEqual(["--extension", extensionDir]);
        }),
      ),
    ),
  );

  it("returns no paths when settings or packages are missing", () => {
    expect(resolvePrimePackageCatalogExtensionPaths("/definitely/not/a-prime-agent-dir")).toEqual(
      [],
    );
  });
});
