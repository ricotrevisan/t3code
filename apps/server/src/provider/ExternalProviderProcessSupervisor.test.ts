import * as NodeServices from "@effect/platform-node/NodeServices";
import { assert, describe, it } from "@effect/vitest";
import { ThreadId } from "@t3tools/contracts";
import * as Effect from "effect/Effect";
import * as Fiber from "effect/Fiber";
import * as Option from "effect/Option";
import * as Stream from "effect/Stream";

import { vi } from "vite-plus/test";
import { loadDirenvExportedEnv } from "../workspace/workspaceDirenvEnv.ts";

vi.mock("../workspace/workspaceDirenvEnv.ts", () => ({
  loadDirenvExportedEnv: vi.fn(() => ({
    T3_PI_MCP_ENDPOINT: "workspace-endpoint",
    T3_PI_MCP_AUTHORIZATION: "workspace-secret",
    ORDINARY_WORKSPACE_VALUE: "workspace",
  })),
}));

import { makeExternalProviderProcessSupervisor } from "./ExternalProviderProcessSupervisor.ts";

const encoder = new TextEncoder();

describe("ExternalProviderProcessSupervisor", () => {
  it.effect(
    "protects session credentials from direnv and removes them for probes without changing ordinary precedence",
    () =>
      Effect.gen(function* () {
        const supervisor = yield* makeExternalProviderProcessSupervisor();
        for (const granted of [true, false]) {
          const child = yield* supervisor.processes.spawn({
            command: process.execPath,
            cwd: process.cwd(),
            purpose: { kind: "probe" },
            environment: {
              T3_PI_MCP_AUTHORIZATION: "instance-secret",
              ORDINARY_WORKSPACE_VALUE: "instance",
            },
            protectedEnvironment: {
              T3_PI_MCP_AUTHORIZATION: granted ? "session-secret" : undefined,
              T3_PI_MCP_ENDPOINT: granted ? "session-endpoint" : undefined,
            },
            args: [
              "-e",
              "process.stdout.write(JSON.stringify([process.env.T3_PI_MCP_AUTHORIZATION, process.env.T3_PI_MCP_ENDPOINT, process.env.ORDINARY_WORKSPACE_VALUE]));",
            ],
          });
          const output = yield* child.stdout.pipe(Stream.decodeText(), Stream.runCollect);
          assert.equal(
            Array.from(output).join(""),
            granted
              ? '["session-secret","session-endpoint","workspace"]'
              : '[null,null,"workspace"]',
          );
          assert.equal(yield* child.exitCode, 0);
        }
        assert.isTrue(vi.mocked(loadDirenvExportedEnv).mock.calls.length > 0);
      }).pipe(Effect.scoped, Effect.provide(NodeServices.layer)),
  );

  it.effect("owns stdio, exit, and idempotent shutdown for an adapter process", () =>
    Effect.gen(function* () {
      const supervisor = yield* makeExternalProviderProcessSupervisor();
      const child = yield* supervisor.processes.spawn({
        command: process.execPath,
        purpose: { kind: "probe" },
        args: [
          "-e",
          "process.stdin.on('data', chunk => process.stdout.write(chunk)); process.stdin.on('end', () => process.exit(0));",
        ],
      });
      const stdoutFiber = yield* child.stdout.pipe(
        Stream.decodeText(),
        Stream.runFold(
          () => "",
          (output, chunk) => output + chunk,
        ),
        Effect.forkChild,
      );

      yield* child.write(encoder.encode("hello from adapter"));
      yield* child.close;
      yield* child.close;

      const stdout = yield* Fiber.join(stdoutFiber);
      assert.equal(stdout, "hello from adapter");
      assert.equal(yield* child.exitCode, 0);
      assert.isAbove(child.pid, 0);
    }).pipe(Effect.scoped, Effect.provide(NodeServices.layer)),
  );

  it.effect("refuses writes after the child exits unexpectedly", () =>
    Effect.gen(function* () {
      const supervisor = yield* makeExternalProviderProcessSupervisor();
      const threadId = ThreadId.make("crashed-session");
      const exitFiber = yield* supervisor.unexpectedExits.pipe(
        Stream.take(1),
        Stream.runHead,
        Effect.forkChild,
      );
      const child = yield* supervisor.processes.spawn({
        command: process.execPath,
        purpose: { kind: "session", threadId },
        args: ["-e", "process.exit(17)"],
      });
      assert.equal(yield* child.exitCode, 17);
      const exit = Option.getOrThrow(yield* Fiber.join(exitFiber));
      assert.deepStrictEqual(exit, { pid: child.pid, threadId, exitCode: 17 });
      const result = yield* child.write(encoder.encode("late")).pipe(Effect.result);

      assert.equal(result._tag, "Failure");
      if (result._tag === "Failure") {
        assert.equal(result.failure.operation, "stdin");
      }
    }).pipe(Effect.scoped, Effect.provide(NodeServices.layer)),
  );

  it.effect("refuses writes after the host closes the process", () =>
    Effect.gen(function* () {
      const supervisor = yield* makeExternalProviderProcessSupervisor();
      const child = yield* supervisor.processes.spawn({
        command: process.execPath,
        purpose: { kind: "probe" },
        args: ["-e", "process.stdin.resume(); process.stdin.on('end', () => process.exit(0));"],
      });
      yield* child.close;
      const result = yield* child.write(encoder.encode("late")).pipe(Effect.result);

      assert.equal(result._tag, "Failure");
      if (result._tag === "Failure") {
        assert.equal(result.failure._tag, "ProviderAdapterHostProcessError");
        assert.equal(result.failure.operation, "stdin");
      }
    }).pipe(Effect.scoped, Effect.provide(NodeServices.layer)),
  );
});
