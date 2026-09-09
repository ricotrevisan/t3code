// @effect-diagnostics nodeBuiltinImport:off
import * as NodePath from "node:path";
import * as NodeURL from "node:url";

import * as NodeServices from "@effect/platform-node/NodeServices";
import { it } from "@effect/vitest";
import * as Effect from "effect/Effect";
import * as FileSystem from "effect/FileSystem";
import { describe, expect } from "vite-plus/test";

import {
  isPrimeApprovalExtensionHandshake,
  preparePrimeApprovalExtension,
  PRIME_APPROVAL_EXTENSION_COMMAND_DESCRIPTION,
  PRIME_APPROVAL_EXTENSION_COMMAND_NAME,
  PRIME_APPROVAL_EXTENSION_PROTOCOL_VERSION,
} from "./primeApprovalExtension.ts";

const provide = <A, E, R>(effect: Effect.Effect<A, E, R>) =>
  effect.pipe(Effect.provide(NodeServices.layer));

type ToolCallHandler = (
  event: { readonly toolName: string; readonly input: Record<string, unknown> },
  context: {
    readonly hasUI: boolean;
    readonly signal?: AbortSignal;
    readonly ui: {
      readonly confirm: (
        title: string,
        message: string,
        options?: { readonly signal?: AbortSignal; readonly timeout?: number },
      ) => Promise<boolean>;
    };
  },
) => Promise<unknown>;

interface ExtensionHarness {
  readonly flag: {
    readonly name: string;
    readonly options: { readonly type: string; readonly default?: boolean | string };
  };
  readonly command: {
    readonly name: string;
    readonly description?: string;
  };
  readonly setMode: (mode: string | boolean | undefined) => void;
  readonly toolCall: ToolCallHandler;
}

const loadExtensionHarness = (extensionPath: string, mode: string | boolean | undefined) =>
  Effect.gen(function* () {
    let currentMode = mode;
    let flag: ExtensionHarness["flag"] | undefined;
    let command: ExtensionHarness["command"] | undefined;
    let toolCall: ToolCallHandler | undefined;
    const loaded = yield* Effect.tryPromise(
      () => import(NodeURL.pathToFileURL(extensionPath).href),
    );
    const factory = loaded.default as (api: {
      registerFlag(
        name: string,
        options: { readonly type: string; readonly default?: boolean | string },
      ): void;
      registerCommand(name: string, options: { readonly description?: string }): void;
      getFlag(name: string): string | boolean | undefined;
      on(event: "tool_call", handler: ToolCallHandler): void;
    }) => void;
    factory({
      registerFlag: (name, options) => {
        flag = { name, options };
      },
      registerCommand: (name, options) => {
        command = {
          name,
          ...(options.description !== undefined ? { description: options.description } : {}),
        };
      },
      getFlag: () => currentMode,
      on: (_event, handler) => {
        toolCall = handler;
      },
    });
    if (flag === undefined || command === undefined || toolCall === undefined) {
      return yield* Effect.die(new Error("Installed extension did not register its protocol."));
    }
    return {
      flag,
      command,
      setMode: (nextMode) => {
        currentMode = nextMode;
      },
      toolCall,
    } satisfies ExtensionHarness;
  });

describe("primeApprovalExtension", () => {
  it.live("installs the versioned extension at a deterministic absolute path", () =>
    provide(
      Effect.scoped(
        Effect.gen(function* () {
          const fileSystem = yield* FileSystem.FileSystem;
          const baseDir = yield* fileSystem.makeTempDirectoryScoped({
            prefix: "t3-prime-approval-extension-",
          });

          const firstPath = yield* preparePrimeApprovalExtension(baseDir);
          const firstSource = yield* fileSystem.readFileString(firstPath);
          const secondPath = yield* preparePrimeApprovalExtension(baseDir);
          const secondSource = yield* fileSystem.readFileString(secondPath);

          expect(firstPath).toBe(
            NodePath.join(baseDir, "prime-agent", "extensions", "t3-approval-v1.ts"),
          );
          expect(NodePath.isAbsolute(firstPath)).toBe(true);
          expect(secondPath).toBe(firstPath);
          expect(secondSource).toBe(firstSource);
          expect(PRIME_APPROVAL_EXTENSION_PROTOCOL_VERSION).toBe(1);
          expect(PRIME_APPROVAL_EXTENSION_COMMAND_NAME).toBe("t3-approval-v1");
          expect(PRIME_APPROVAL_EXTENSION_COMMAND_DESCRIPTION).toBe(
            "T3 approval bridge protocol 1",
          );
        }),
      ),
    ),
  );

  it.live("loads as a standalone module and registers the handshake protocol", () =>
    provide(
      Effect.scoped(
        Effect.gen(function* () {
          const fileSystem = yield* FileSystem.FileSystem;
          const baseDir = yield* fileSystem.makeTempDirectoryScoped({
            prefix: "t3-prime-approval-extension-protocol-",
          });
          const extensionPath = yield* preparePrimeApprovalExtension(baseDir);
          const source = yield* fileSystem.readFileString(extensionPath);
          const harness = yield* loadExtensionHarness(extensionPath, "approval-required");

          expect(
            source
              .split("\n")
              .filter((line) => /^import\s/.test(line) && !/^import\s+type\s/.test(line)),
          ).toEqual([]);
          expect(harness.flag).toMatchObject({
            name: "t3-approval-mode",
            options: { type: "string", default: "deny" },
          });
          expect(harness.command).toEqual({
            name: "t3-approval-v1",
            description: "T3 approval bridge protocol 1",
          });
        }),
      ),
    ),
  );

  it.live("reads the mode after Prime applies CLI flag values", () =>
    provide(
      Effect.scoped(
        Effect.gen(function* () {
          const fileSystem = yield* FileSystem.FileSystem;
          const baseDir = yield* fileSystem.makeTempDirectoryScoped({
            prefix: "t3-prime-approval-extension-late-flag-",
          });
          const extensionPath = yield* preparePrimeApprovalExtension(baseDir);
          const harness = yield* loadExtensionHarness(extensionPath, "deny");
          let confirmations = 0;

          harness.setMode("approval-required");
          const result = yield* Effect.promise(() =>
            harness.toolCall(
              { toolName: "ipython", input: { code: "1 + 1" } },
              {
                hasUI: true,
                ui: {
                  confirm: async () => {
                    confirmations += 1;
                    return true;
                  },
                },
              },
            ),
          );

          expect(result).toBeUndefined();
          expect(confirmations).toBe(1);
        }),
      ),
    ),
  );

  it.live("allows every tool immediately only in full-access mode", () =>
    provide(
      Effect.scoped(
        Effect.gen(function* () {
          const fileSystem = yield* FileSystem.FileSystem;
          const baseDir = yield* fileSystem.makeTempDirectoryScoped({
            prefix: "t3-prime-approval-extension-full-access-",
          });
          const extensionPath = yield* preparePrimeApprovalExtension(baseDir);
          const harness = yield* loadExtensionHarness(extensionPath, "full-access");
          let confirmations = 0;
          const context = {
            hasUI: true,
            ui: {
              confirm: async () => {
                confirmations += 1;
                return false;
              },
            },
          };

          expect(
            yield* Effect.promise(() =>
              harness.toolCall({ toolName: "ipython", input: { code: "1 + 1" } }, context),
            ),
          ).toBeUndefined();
          expect(
            yield* Effect.promise(() =>
              harness.toolCall({ toolName: "custom-tool", input: { value: 1 } }, context),
            ),
          ).toBeUndefined();
          expect(confirmations).toBe(0);
        }),
      ),
    ),
  );

  it.live("fails closed for missing, deny, and invalid modes", () =>
    provide(
      Effect.scoped(
        Effect.gen(function* () {
          const fileSystem = yield* FileSystem.FileSystem;
          const baseDir = yield* fileSystem.makeTempDirectoryScoped({
            prefix: "t3-prime-approval-extension-invalid-mode-",
          });
          const extensionPath = yield* preparePrimeApprovalExtension(baseDir);

          for (const mode of [undefined, "deny", "unexpected"] as const) {
            const harness = yield* loadExtensionHarness(extensionPath, mode);
            let confirmations = 0;
            const result = yield* Effect.promise(() =>
              harness.toolCall(
                { toolName: "ipython", input: { code: "danger()" } },
                {
                  hasUI: true,
                  ui: {
                    confirm: async () => {
                      confirmations += 1;
                      return true;
                    },
                  },
                },
              ),
            );
            expect(result).toEqual({
              block: true,
              reason: "T3 approval bridge mode is missing or invalid",
            });
            expect(confirmations).toBe(0);
          }
        }),
      ),
    ),
  );

  it.live("blocks supervised tool calls without UI", () =>
    provide(
      Effect.scoped(
        Effect.gen(function* () {
          const fileSystem = yield* FileSystem.FileSystem;
          const baseDir = yield* fileSystem.makeTempDirectoryScoped({
            prefix: "t3-prime-approval-extension-no-ui-",
          });
          const extensionPath = yield* preparePrimeApprovalExtension(baseDir);
          const harness = yield* loadExtensionHarness(extensionPath, "approval-required");

          const result = yield* Effect.promise(() =>
            harness.toolCall(
              { toolName: "bash", input: { command: "touch marker" } },
              { hasUI: false, ui: { confirm: async () => true } },
            ),
          );
          expect(result).toEqual({
            block: true,
            reason: "T3 approval UI is unavailable",
          });
        }),
      ),
    ),
  );

  it.live("asks once with the turn signal, allows confirmation, and blocks rejection", () =>
    provide(
      Effect.scoped(
        Effect.gen(function* () {
          const fileSystem = yield* FileSystem.FileSystem;
          const baseDir = yield* fileSystem.makeTempDirectoryScoped({
            prefix: "t3-prime-approval-extension-confirm-",
          });
          const extensionPath = yield* preparePrimeApprovalExtension(baseDir);
          const harness = yield* loadExtensionHarness(extensionPath, "approval-required");
          const abortController = new AbortController();
          const calls: Array<{
            title: string;
            message: string;
            options?: { readonly signal?: AbortSignal; readonly timeout?: number };
          }> = [];
          let decision = true;
          const context = {
            hasUI: true,
            signal: abortController.signal,
            ui: {
              confirm: async (
                title: string,
                message: string,
                options?: { readonly signal?: AbortSignal; readonly timeout?: number },
              ) => {
                calls.push({
                  title,
                  message,
                  ...(options !== undefined ? { options } : {}),
                });
                return decision;
              },
            },
          };

          const event = { toolName: "ipython", input: { code: "run_command()" } };
          expect(yield* Effect.promise(() => harness.toolCall(event, context))).toBeUndefined();
          expect(calls).toEqual([
            {
              title: "Allow Prime Agent tool?",
              message: 'ipython\n\n{\n  "code": "run_command()"\n}',
              options: { signal: abortController.signal },
            },
          ]);
          expect(calls[0]?.options).not.toHaveProperty("timeout");

          decision = false;
          expect(yield* Effect.promise(() => harness.toolCall(event, context))).toEqual({
            block: true,
            reason: "Rejected by user",
          });
        }),
      ),
    ),
  );

  it("validates the exact RPC handshake command and source identity", () => {
    const extensionPath = "/t3-home/prime-agent/extensions/t3-approval-v1.ts";
    const command = {
      name: "t3-approval-v1",
      description: "T3 approval bridge protocol 1",
      source: "extension",
      sourceInfo: {
        path: extensionPath,
        source: "cli",
        scope: "temporary",
        origin: "top-level",
      },
    };

    expect(isPrimeApprovalExtensionHandshake({ commands: [command] }, extensionPath)).toBe(true);
    expect(isPrimeApprovalExtensionHandshake({ commands: [command, command] }, extensionPath)).toBe(
      false,
    );
    expect(
      isPrimeApprovalExtensionHandshake(
        { commands: [{ ...command, description: "wrong protocol" }] },
        extensionPath,
      ),
    ).toBe(false);
    expect(
      isPrimeApprovalExtensionHandshake(
        {
          commands: [
            {
              ...command,
              sourceInfo: { ...command.sourceInfo, path: "/other/extension.ts" },
            },
          ],
        },
        extensionPath,
      ),
    ).toBe(false);
    expect(isPrimeApprovalExtensionHandshake({ commands: "malformed" }, extensionPath)).toBe(false);
  });
});
