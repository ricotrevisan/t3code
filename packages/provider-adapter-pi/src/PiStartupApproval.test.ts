// @effect-diagnostics nodeBuiltinImport:off
import { describe, expect, it } from "@effect/vitest";
import { ApprovalRequestId, ProviderInstanceId, ThreadId } from "@t3tools/contracts";
import type { ProviderAdapterHostV2 } from "@t3tools/provider-adapter";
import * as Deferred from "effect/Deferred";
import * as Effect from "effect/Effect";
import * as Fiber from "effect/Fiber";
import * as Queue from "effect/Queue";
import * as Stream from "effect/Stream";
import * as TestClock from "effect/testing/TestClock";
import { makePiAdapter } from "./PiAdapter.ts";

describe("Pi startup approval", () => {
  for (const scenario of [
    "immediate",
    "long-wait",
    "denied",
    "cancelled",
    "missing-bridge",
    "unresponsive",
  ] as const) {
    it.effect(`keeps preview startup safe during ${scenario} approval`, () =>
      Effect.gen(function* () {
        const output = yield* Queue.unbounded<Uint8Array>();
        let healthId: string | undefined;
        let startupSettled = false;
        let closed = false;
        const emit = (value: unknown) =>
          Queue.offer(output, new TextEncoder().encode(JSON.stringify(value) + "\n"));
        const exited = yield* Deferred.make<number>();
        const close = Effect.sync(() => {
          closed = true;
        }).pipe(Effect.andThen(Deferred.succeed(exited, 0)), Effect.asVoid);
        const host: ProviderAdapterHostV2 = {
          protocolVersion: 2,
          processes: {
            spawn: () =>
              Effect.acquireRelease(
                Effect.succeed({
                  pid: 1,
                  attachSession: () => Effect.void,
                  detachSession: () => Effect.void,
                  expectExit: Effect.void,
                  exitCode: Deferred.await(exited),
                  close,
                  stdout: Stream.fromQueue(output),
                  stderr: Stream.empty,
                  write: (chunk) =>
                    Effect.gen(function* () {
                      const command = JSON.parse(new TextDecoder().decode(chunk));
                      if (command.type === "get_commands") {
                        healthId = command.id;
                        if (scenario === "unresponsive") return;
                        yield* emit({
                          type: "extension_ui_request",
                          id: "startup-mcp",
                          method: "confirm",
                          title: "Project MCP approval",
                          message: "Allow fixture MCP?",
                        });
                      } else if (command.type === "extension_ui_response") {
                        expect(command.confirmed).toBe(scenario !== "denied");
                        yield* emit({
                          type: "response",
                          id: healthId,
                          command: "get_commands",
                          success: true,
                          data: {
                            commands:
                              scenario === "missing-bridge"
                                ? []
                                : [{ name: "t3-preview-health", source: "extension" }],
                          },
                        });
                      } else {
                        yield* emit({
                          type: "response",
                          id: command.id,
                          command: command.type,
                          success: true,
                          data: {
                            sessionFile: "/fixture/session.jsonl",
                            sessionId: "native",
                            isStreaming: false,
                            isCompacting: false,
                          },
                        });
                      }
                    }),
                }),
                () => close,
              ),
          },
          workspaces: { resolveCwd: () => Effect.succeed("/fixture") },
          storage: {
            prepareSession: () =>
              Effect.succeed({ sessionDirectory: "/fixture", sharedDirectory: "/fixture" }),
            validateSessionFile: (input) => Effect.succeed(input.path),
            materializeArtifact: () => Effect.succeed("/fixture/bridge.mjs"),
          },
          attachments: { read: () => Effect.die("unused") },
          mcp: {
            readSession: () =>
              Effect.succeed({
                endpoint: "http://localhost/mcp",
                authorizationHeader: "Bearer synthetic",
                capabilities: new Set(["preview"]),
              }),
          },
        };
        const config = { binaryPath: "pi", args: [] };
        const instance = yield* makePiAdapter(
          config,
          {
            config,
            instanceId: ProviderInstanceId.make("pi-startup"),
            displayName: undefined,
            accentColor: undefined,
            environment: {},
            enabled: true,
          },
          host,
        );
        const events = yield* instance.adapter.streamEvents.pipe(
          Stream.toQueue({ capacity: "unbounded" }),
        );
        const threadId = ThreadId.make("startup");
        const startup = yield* instance.adapter
          .startSession({ threadId, runtimeMode: "full-access" })
          .pipe(
            Effect.result,
            Effect.tap(() =>
              Effect.sync(() => {
                startupSettled = true;
              }),
            ),
            Effect.forkChild,
          );
        if (scenario === "unresponsive") {
          yield* TestClock.adjust("31 seconds");
          const result = yield* Fiber.join(startup);
          expect(result._tag).toBe("Failure");
          if (result._tag === "Failure")
            expect(result.failure.detail).toContain("T3 preview bridge did not initialize");
          expect(closed).toBe(true);
          expect(yield* instance.adapter.hasSession(threadId)).toBe(false);
          return;
        }
        const event = yield* Queue.take(events);
        expect(event.type).toBe("user-input.requested");
        expect(yield* instance.adapter.hasSession(threadId)).toBe(false);
        expect(yield* instance.adapter.listSessions()).toEqual([]);
        if (scenario === "long-wait") {
          yield* TestClock.adjust("61 seconds");
          yield* Effect.yieldNow;
        }
        expect(startupSettled).toBe(false);
        expect(closed).toBe(false);
        const requestId = ApprovalRequestId.make("startup-mcp");
        if (scenario === "cancelled") {
          yield* Fiber.interrupt(startup);
          expect(closed).toBe(true);
          expect(yield* instance.adapter.hasSession(threadId)).toBe(false);
          const resolved = yield* Queue.take(events);
          expect(resolved.type).toBe("user-input.resolved");
          const late = yield* instance.adapter
            .respondToUserInput(threadId, requestId, { "startup-mcp": "Yes" })
            .pipe(Effect.result);
          expect(late._tag).toBe("Failure");
          return;
        }
        yield* instance.adapter.respondToUserInput(threadId, requestId, {
          "startup-mcp": scenario === "denied" ? "No" : "Yes",
        });
        const result = yield* Fiber.join(startup);
        if (scenario === "missing-bridge") {
          expect(result._tag).toBe("Failure");
          if (result._tag === "Failure")
            expect(result.failure.detail).toContain("T3 preview bridge did not initialize");
          expect(yield* instance.adapter.hasSession(threadId)).toBe(false);
          expect(closed).toBe(true);
          return;
        }
        expect(result._tag).toBe("Success");
        expect(yield* instance.adapter.hasSession(threadId)).toBe(true);
        yield* instance.adapter.stopSession(threadId);
        expect(closed).toBe(true);
      }).pipe(Effect.scoped),
    );
  }
});
