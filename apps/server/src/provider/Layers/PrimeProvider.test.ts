// @effect-diagnostics nodeBuiltinImport:off
import * as NodePath from "node:path";
import * as NodeURL from "node:url";

import * as NodeServices from "@effect/platform-node/NodeServices";
import { describe, expect, it } from "@effect/vitest";
import { PrimeSettings } from "@t3tools/contracts";
import * as Effect from "effect/Effect";
import * as FileSystem from "effect/FileSystem";
import * as Path from "effect/Path";
import * as Schema from "effect/Schema";

import {
  loggedInPrimeProvidersFromAuthData,
  mapPrimeAvailableModels,
} from "../prime/primeModels.ts";
import { buildInitialPrimeProviderSnapshot, checkPrimeProviderStatus } from "./PrimeProvider.ts";

const decodePrimeSettings = Schema.decodeSync(PrimeSettings);

const mockAgentPath = NodePath.join(
  NodePath.dirname(NodeURL.fileURLToPath(import.meta.url)),
  "../../scripts/prime-rpc-mock-agent.ts",
);

const PATH_TRAP_ENV: NodeJS.ProcessEnv = {
  ...process.env,
  PATH: "/definitely/not/a/prime-agent-path",
  PRIME_AGENT_CODING_AGENT_DIR: "/definitely/not/a-prime-agent-dir",
};

function shSingleQuote(value: string): string {
  return `'${value.replaceAll("'", "'\\''")}'`;
}

type ApprovalHandshakeFixtureKind =
  | "valid"
  | "missing"
  | "malformed"
  | "wrong-path"
  | "wrong-source"
  | "failure";

const makeApprovalHandshakeFixture = Effect.fn("makeApprovalHandshakeFixture")(function* (
  kind: ApprovalHandshakeFixtureKind,
) {
  const fs = yield* FileSystem.FileSystem;
  const path = yield* Path.Path;
  const dir = yield* fs.makeTempDirectoryScoped({ prefix: "t3code-prime-approval-probe-" });
  const binaryPath = path.join(dir, "prime-agent");
  const approvalExtensionBaseDir = path.join(dir, "t3-home");
  const requestLogPath = path.join(dir, "requests.jsonl");
  yield* fs.writeFileString(
    binaryPath,
    [
      "#!/bin/sh",
      ...(kind === "failure"
        ? ['for arg in "$@"; do [ "$arg" = "--extension" ] && exit 2; done']
        : []),
      `exec ${shSingleQuote(process.execPath)} ${shSingleQuote(mockAgentPath)} "$@"`,
      "",
    ].join("\n"),
  );
  yield* fs.chmod(binaryPath, 0o755);
  return { approvalExtensionBaseDir, binaryPath, kind, requestLogPath };
});

describe("mapPrimeAvailableModels", () => {
  it("maps Prime's sparse thinking-level map semantics", () => {
    const models = mapPrimeAvailableModels([
      {
        id: "gpt-5.6-sol",
        name: "GPT-5.6 Sol",
        provider: "openai-codex",
        thinkingLevelMap: { xhigh: "xhigh", minimal: null, max: "max" },
      },
      {
        id: "orphan",
        name: "Missing provider",
      },
    ]);
    expect(models).toHaveLength(1);
    expect(models[0]?.slug).toBe("openai-codex/gpt-5.6-sol");
    expect(models[0]?.subProvider).toBe("openai-codex");
    const descriptor = models[0]?.capabilities?.optionDescriptors?.[0];
    expect(descriptor?.id).toBe("thinkingLevel");
    expect(
      descriptor?.type === "select" ? descriptor.options.map((option) => option.id) : [],
    ).toEqual(["off", "low", "medium", "high", "xhigh", "max"]);
  });

  it("hides models whose Prime provider is not logged in", () => {
    const models = mapPrimeAvailableModels(
      [
        {
          id: "gpt-5.6-sol",
          name: "GPT-5.6 Sol",
          provider: "openai-codex",
        },
        {
          id: "claude-fable-5",
          name: "Claude Fable 5",
          provider: "prime-inference",
        },
      ],
      { loggedInProviders: new Set(["openai-codex"]) },
    );
    expect(models.map((model) => model.slug)).toEqual(["openai-codex/gpt-5.6-sol"]);
  });
});

describe("loggedInPrimeProvidersFromAuthData", () => {
  it("keeps stored oauth and api_key providers and ignores other keys", () => {
    const providers = loggedInPrimeProvidersFromAuthData({
      "openai-codex": { type: "oauth" },
      "opencode-go": { type: "api_key" },
      "prime-inference": { note: "ambient" },
      "": { type: "oauth" },
    });
    expect([...providers].toSorted()).toEqual(["openai-codex", "opencode-go"]);
  });
});

describe("buildInitialPrimeProviderSnapshot", () => {
  it.effect("returns a disabled snapshot when settings.enabled is false", () =>
    Effect.gen(function* () {
      const snapshot = yield* buildInitialPrimeProviderSnapshot(
        decodePrimeSettings({ enabled: false }),
      );
      expect(snapshot.enabled).toBe(false);
      expect(snapshot.status).toBe("disabled");
      expect(snapshot.installed).toBe(false);
      expect(snapshot.message).toContain("disabled");
      expect(snapshot.supportedRuntimeModes).toEqual(["full-access"]);
    }),
  );

  it.effect("returns a disabled snapshot by default — Prime Agent is opt-in", () =>
    Effect.gen(function* () {
      const snapshot = yield* buildInitialPrimeProviderSnapshot(decodePrimeSettings({}));
      expect(snapshot.enabled).toBe(false);
      expect(snapshot.status).toBe("disabled");
      expect(snapshot.supportedRuntimeModes).toEqual(["full-access"]);
    }),
  );

  it.effect("returns a pending snapshot when enabled", () =>
    Effect.gen(function* () {
      const snapshot = yield* buildInitialPrimeProviderSnapshot(
        decodePrimeSettings({ enabled: true }),
      );
      expect(snapshot.enabled).toBe(true);
      expect(snapshot.installed).toBe(true);
      expect(snapshot.status).toBe("warning");
      expect(snapshot.version).toBeNull();
      expect(snapshot.message).toContain("Checking Prime Agent");
      expect(snapshot.supportedRuntimeModes).toEqual(["full-access"]);
    }),
  );
});

it.layer(NodeServices.layer)("checkPrimeProviderStatus", (it) => {
  it.effect("reports the binary as missing when the binary path does not resolve", () =>
    Effect.gen(function* () {
      const snapshot = yield* checkPrimeProviderStatus(
        decodePrimeSettings({
          enabled: true,
          binaryPath: "/definitely/not/installed/prime-agent",
        }),
        PATH_TRAP_ENV,
      );
      expect(snapshot.enabled).toBe(true);
      expect(snapshot.installed).toBe(false);
      expect(snapshot.status).toBe("error");
      expect(snapshot.version).toBeNull();
      expect(snapshot.message).toMatch(/not installed|not on PATH/);
    }),
  );

  it.effect("does not probe PATH prime-agent when binaryPath is an explicit missing file", () =>
    Effect.gen(function* () {
      const snapshot = yield* Effect.scoped(
        Effect.gen(function* () {
          const fs = yield* FileSystem.FileSystem;
          const path = yield* Path.Path;
          const dir = yield* fs.makeTempDirectoryScoped({ prefix: "t3code-prime-path-trap-" });
          const trapPath = path.join(dir, "prime-agent");
          const markerPath = path.join(dir, "invoked");
          yield* fs.writeFileString(
            trapPath,
            ["#!/bin/sh", `printf invoked > ${shSingleQuote(markerPath)}`, "exit 0", ""].join("\n"),
          );
          yield* fs.chmod(trapPath, 0o755);

          return yield* checkPrimeProviderStatus(
            decodePrimeSettings({
              enabled: true,
              binaryPath: "/definitely/not/installed/prime-agent",
            }),
            { ...process.env, PATH: dir },
          ).pipe(
            Effect.tap(() =>
              fs.exists(markerPath).pipe(
                Effect.flatMap((exists) => {
                  expect(exists).toBe(false);
                  return Effect.void;
                }),
              ),
            ),
          );
        }),
      );

      expect(snapshot.installed).toBe(false);
      expect(snapshot.status).toBe("error");
    }),
  );

  it.effect("reports the version string from a fixture binary --version", () =>
    Effect.gen(function* () {
      const snapshot = yield* Effect.scoped(
        Effect.gen(function* () {
          const fs = yield* FileSystem.FileSystem;
          const path = yield* Path.Path;
          const dir = yield* fs.makeTempDirectoryScoped({ prefix: "t3code-prime-version-" });
          const binaryPath = path.join(dir, "prime-agent");
          yield* fs.writeFileString(
            binaryPath,
            [
              "#!/bin/sh",
              `exec ${shSingleQuote(process.execPath)} ${shSingleQuote(mockAgentPath)} "$@"`,
              "",
            ].join("\n"),
          );
          yield* fs.chmod(binaryPath, 0o755);

          return yield* checkPrimeProviderStatus(
            decodePrimeSettings({ enabled: true, binaryPath }),
            PATH_TRAP_ENV,
            { approvalExtensionBaseDir: path.join(dir, "t3-home") },
          );
        }),
      );

      expect(snapshot.enabled).toBe(true);
      expect(snapshot.installed).toBe(true);
      expect(snapshot.status).toBe("ready");
      expect(snapshot.version).toBe("0.0.1");
      expect(snapshot.auth.status).toBe("authenticated");
      expect(snapshot.requiresNewThreadForModelChange).toBe(false);
      expect(snapshot.supportedRuntimeModes).toEqual(["full-access", "approval-required"]);
      expect(snapshot.models.map((model) => model.slug)).toEqual([
        "anthropic/claude-sonnet-4",
        "openai-codex/gpt-5.6-sol",
        "local/no-think",
        "openrouter/stealth/ox-alpha",
      ]);
      const sol = snapshot.models.find((model) => model.slug === "openai-codex/gpt-5.6-sol");
      expect(sol?.capabilities?.optionDescriptors?.[0]?.id).toBe("thinkingLevel");
      expect(
        sol?.capabilities?.optionDescriptors?.[0]?.type === "select"
          ? sol.capabilities.optionDescriptors[0].options.map((option) => option.id)
          : [],
      ).toEqual(["off", "low", "medium", "high", "xhigh", "max"]);
    }),
  );

  it.effect("hides models for Prime providers that are not in auth.json", () =>
    Effect.gen(function* () {
      const snapshot = yield* Effect.scoped(
        Effect.gen(function* () {
          const fs = yield* FileSystem.FileSystem;
          const path = yield* Path.Path;
          const dir = yield* fs.makeTempDirectoryScoped({ prefix: "t3code-prime-version-" });
          const agentDir = yield* fs.makeTempDirectoryScoped({ prefix: "t3code-prime-auth-" });
          const binaryPath = path.join(dir, "prime-agent");
          yield* fs.writeFileString(
            binaryPath,
            [
              "#!/bin/sh",
              `exec ${shSingleQuote(process.execPath)} ${shSingleQuote(mockAgentPath)} "$@"`,
              "",
            ].join("\n"),
          );
          yield* fs.chmod(binaryPath, 0o755);
          yield* fs.writeFileString(
            path.join(agentDir, "auth.json"),
            '{"openai-codex":{"type":"oauth"}}',
          );

          return yield* checkPrimeProviderStatus(
            decodePrimeSettings({ enabled: true, binaryPath }),
            {
              ...PATH_TRAP_ENV,
              PRIME_AGENT_CODING_AGENT_DIR: agentDir,
            },
          );
        }),
      );

      expect(snapshot.status).toBe("ready");
      expect(snapshot.auth.status).toBe("authenticated");
      expect(snapshot.models.map((model) => model.slug)).toEqual(["openai-codex/gpt-5.6-sol"]);
    }),
  );

  it.effect("forwards installed Prime package catalogs when listing models", () =>
    Effect.gen(function* () {
      const snapshot = yield* Effect.scoped(
        Effect.gen(function* () {
          const fs = yield* FileSystem.FileSystem;
          const path = yield* Path.Path;
          const dir = yield* fs.makeTempDirectoryScoped({ prefix: "t3code-prime-package-list-" });
          const agentDir = yield* fs.makeTempDirectoryScoped({
            prefix: "t3code-prime-package-agent-",
          });
          const binaryPath = path.join(dir, "prime-agent");
          const requestLogPath = path.join(dir, "requests.jsonl");
          const xaiRoot = path.join(agentDir, "npm", "node_modules", "pi-xai-oauth");
          const xaiExtension = path.join(xaiRoot, "extensions", "xai-oauth.ts");
          yield* fs.makeDirectory(path.join(xaiRoot, "extensions"), { recursive: true });
          yield* fs.writeFileString(
            path.join(xaiRoot, "package.json"),
            // @effect-diagnostics-next-line preferSchemaOverJson:off - fixed package manifest fixture.
            JSON.stringify({
              name: "pi-xai-oauth",
              pi: { extensions: ["./extensions"] },
            }),
          );
          yield* fs.writeFileString(xaiExtension, "export default async function () {}");
          yield* fs.writeFileString(
            path.join(agentDir, "settings.json"),
            // @effect-diagnostics-next-line preferSchemaOverJson:off - fixed Prime settings fixture.
            JSON.stringify({ packages: ["npm:pi-xai-oauth"] }),
          );
          yield* fs.writeFileString(
            binaryPath,
            [
              "#!/bin/sh",
              `exec ${shSingleQuote(process.execPath)} ${shSingleQuote(mockAgentPath)} "$@"`,
              "",
            ].join("\n"),
          );
          yield* fs.chmod(binaryPath, 0o755);

          const result = yield* checkPrimeProviderStatus(
            decodePrimeSettings({ enabled: true, binaryPath }),
            {
              ...PATH_TRAP_ENV,
              PRIME_AGENT_CODING_AGENT_DIR: agentDir,
              T3_PRIME_MOCK_REQUEST_LOG_PATH: requestLogPath,
            },
            { approvalExtensionBaseDir: path.join(dir, "t3-home") },
          );

          const invocations = (yield* fs.readFileString(requestLogPath))
            .trim()
            .split("\n")
            .map(
              (line) => JSON.parse(line) as { args: Array<string>; command: { type?: unknown } },
            );
          const listArgs = invocations.find(
            (entry) => entry.command.type === "get_available_models",
          )?.args;
          expect(listArgs).toBeDefined();
          expect(listArgs).toContain("--no-extensions");
          const extensionPaths = (listArgs ?? []).flatMap((arg, index, args) =>
            arg === "--extension" && args[index + 1] ? [args[index + 1]!] : [],
          );
          expect(extensionPaths).toContain(xaiExtension);
          expect(extensionPaths.some((value) => value.endsWith("t3-openrouter-catalog.ts"))).toBe(
            true,
          );

          return result;
        }),
      );

      expect(snapshot.status).toBe("ready");
      expect(snapshot.models.length).toBeGreaterThan(0);
    }),
  );

  it.effect("does not fall back to a hardcoded model list when listing fails", () =>
    Effect.gen(function* () {
      const snapshot = yield* Effect.scoped(
        Effect.gen(function* () {
          const fs = yield* FileSystem.FileSystem;
          const path = yield* Path.Path;
          const dir = yield* fs.makeTempDirectoryScoped({ prefix: "t3code-prime-models-fail-" });
          const binaryPath = path.join(dir, "prime-agent");
          yield* fs.writeFileString(
            binaryPath,
            [
              "#!/bin/sh",
              'if [ "$1" = "--version" ]; then printf "0.0.9\\n"; exit 0; fi',
              "exit 1",
              "",
            ].join("\n"),
          );
          yield* fs.chmod(binaryPath, 0o755);

          return yield* checkPrimeProviderStatus(
            decodePrimeSettings({ enabled: true, binaryPath }),
            PATH_TRAP_ENV,
          );
        }),
      );

      expect(snapshot.installed).toBe(true);
      expect(snapshot.version).toBe("0.0.9");
      expect(snapshot.status).toBe("warning");
      expect(snapshot.models).toEqual([]);
      expect(snapshot.message).toContain("could not list models");
    }),
  );

  it.effect("reports an installed CLI as unhealthy when --version exits non-zero", () =>
    Effect.gen(function* () {
      const snapshot = yield* Effect.scoped(
        Effect.gen(function* () {
          const fs = yield* FileSystem.FileSystem;
          const path = yield* Path.Path;
          const dir = yield* fs.makeTempDirectoryScoped({ prefix: "t3code-prime-version-fail-" });
          const binaryPath = path.join(dir, "prime-agent");
          yield* fs.writeFileString(
            binaryPath,
            ["#!/bin/sh", 'printf "broken prime install\\n" >&2', "exit 2", ""].join("\n"),
          );
          yield* fs.chmod(binaryPath, 0o755);

          return yield* checkPrimeProviderStatus(
            decodePrimeSettings({ enabled: true, binaryPath }),
            PATH_TRAP_ENV,
          );
        }),
      );

      expect(snapshot.enabled).toBe(true);
      expect(snapshot.installed).toBe(true);
      expect(snapshot.status).toBe("error");
      expect(snapshot.message).toBe("Prime Agent is installed but failed to run.");
      expect(snapshot.message).not.toContain("broken prime install");
    }),
  );

  it.effect("advertises approval-required only after the exact extension handshake", () =>
    Effect.scoped(
      Effect.gen(function* () {
        const fs = yield* FileSystem.FileSystem;
        const path = yield* Path.Path;
        const fixture = yield* makeApprovalHandshakeFixture("valid");
        const snapshot = yield* checkPrimeProviderStatus(
          decodePrimeSettings({ enabled: true, binaryPath: fixture.binaryPath }),
          {
            ...PATH_TRAP_ENV,
            T3_PRIME_MOCK_APPROVAL_HANDSHAKE: fixture.kind,
            T3_PRIME_MOCK_REQUEST_LOG_PATH: fixture.requestLogPath,
          },
          { approvalExtensionBaseDir: fixture.approvalExtensionBaseDir },
        );

        expect(snapshot.status).toBe("ready");
        expect(snapshot.models.length).toBeGreaterThan(0);
        expect(snapshot.supportedRuntimeModes).toEqual(["full-access", "approval-required"]);
        expect(snapshot.showInteractionModeToggle).toBe(false);

        const invocations = (yield* fs.readFileString(fixture.requestLogPath))
          .trim()
          .split("\n")
          .map((line) => JSON.parse(line) as { args: Array<string>; command: { type?: unknown } });
        const handshakeArgs = invocations.find(
          (entry) => entry.command.type === "get_commands",
        )?.args;
        const extensionPath = path.resolve(
          fixture.approvalExtensionBaseDir,
          "prime-agent",
          "extensions",
          "t3-approval-v1.ts",
        );
        expect(handshakeArgs).toEqual([
          "--mode",
          "rpc",
          "--no-session",
          "--no-tools",
          "--no-extensions",
          "--extension",
          extensionPath,
          "--t3-approval-mode=approval-required",
        ]);
      }),
    ),
  );

  for (const kind of ["missing", "malformed", "wrong-path", "wrong-source"] as const) {
    it.effect(
      `keeps model health usable and advertises full-access only for ${kind} handshake`,
      () =>
        Effect.scoped(
          Effect.gen(function* () {
            const fixture = yield* makeApprovalHandshakeFixture(kind);
            const snapshot = yield* checkPrimeProviderStatus(
              decodePrimeSettings({ enabled: true, binaryPath: fixture.binaryPath }),
              {
                ...PATH_TRAP_ENV,
                T3_PRIME_MOCK_APPROVAL_HANDSHAKE: fixture.kind,
                T3_PRIME_MOCK_REQUEST_LOG_PATH: fixture.requestLogPath,
              },
              { approvalExtensionBaseDir: fixture.approvalExtensionBaseDir },
            );

            expect(snapshot.status).toBe("ready");
            expect(snapshot.auth.status).toBe("authenticated");
            expect(snapshot.models.length).toBeGreaterThan(0);
            expect(snapshot.supportedRuntimeModes).toEqual(["full-access"]);
            expect(snapshot.showInteractionModeToggle).toBe(false);
          }),
        ),
    );
  }
});
