// @effect-diagnostics nodeBuiltinImport:off
import * as NodePath from "node:path";
import * as NodeURL from "node:url";

import * as NodeServices from "@effect/platform-node/NodeServices";
import type { ProviderAdapterProcessV1 } from "@t3tools/provider-adapter";
import { ProviderAdapterHostProcessError } from "@t3tools/provider-adapter";
import { it } from "@effect/vitest";
import * as Deferred from "effect/Deferred";
import * as Effect from "effect/Effect";
import * as FileSystem from "effect/FileSystem";
import * as Fiber from "effect/Fiber";
import * as Queue from "effect/Queue";
import * as Ref from "effect/Ref";
import * as Schema from "effect/Schema";
import * as Stream from "effect/Stream";
import { describe, expect } from "vite-plus/test";

import { makeExternalProviderProcessSupervisor } from "../ExternalProviderProcessSupervisor.ts";
import { makePrimeRpcClient, PrimeRpcError } from "@t3tools/provider-adapter-prime";

const mockAgentPath = NodePath.join(
  NodePath.dirname(NodeURL.fileURLToPath(import.meta.url)),
  "../../scripts/prime-rpc-mock-agent.ts",
);

const LINE_SEPARATOR = "\u2028";
const isPrimeRpcError = Schema.is(PrimeRpcError);
const decodeTestCommand = Schema.decodeUnknownSync(
  Schema.fromJsonString(Schema.Struct({ id: Schema.String, type: Schema.String })),
);
const encodeTestJson = Schema.encodeSync(Schema.fromJsonString(Schema.Unknown));

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function eventType(value: unknown): string | undefined {
  return isRecord(value) && typeof value.type === "string" ? value.type : undefined;
}

const spawnSupervisedClient = Effect.fn("spawnSupervisedPrimeRpcTestClient")(function* (input: {
  readonly command: string;
  readonly args: ReadonlyArray<string>;
  readonly cwd: string;
  readonly environment?: NodeJS.ProcessEnv;
}) {
  const supervisor = yield* makeExternalProviderProcessSupervisor();
  const process = yield* supervisor.processes.spawn({
    ...input,
    purpose: { kind: "probe" },
  });
  return yield* makePrimeRpcClient(process);
});

const spawnMock = (environment?: NodeJS.ProcessEnv) =>
  spawnSupervisedClient({
    command: "node",
    args: [mockAgentPath, "--no-session"],
    cwd: process.cwd(),
    ...(environment ? { environment } : {}),
  });

const provide = <A, E, R>(effect: Effect.Effect<A, E, R>) =>
  effect.pipe(Effect.provide(NodeServices.layer));

const makeFakeProcess = Effect.fn("makeFakePrimeProcess")(function* () {
  const stdout = yield* Queue.unbounded<Uint8Array>();
  const stderr = yield* Queue.unbounded<Uint8Array>();
  const written = yield* Queue.unbounded<Uint8Array>();
  const exitCode = yield* Deferred.make<number, ProviderAdapterHostProcessError>();
  const stderrObserved = yield* Queue.unbounded<void>();
  const closeCount = yield* Ref.make(0);
  const textEncoder = new TextEncoder();

  const process: ProviderAdapterProcessV1 = {
    pid: 42,
    attachSession: () => Effect.void,
    detachSession: () => Effect.void,
    expectExit: Effect.void,
    stdout: Stream.fromQueue(stdout),
    stderr: Stream.fromQueue(stderr).pipe(
      Stream.tap(() => Queue.offer(stderrObserved, undefined).pipe(Effect.asVoid)),
    ),
    exitCode: Deferred.await(exitCode),
    write: (chunk) => Queue.offer(written, chunk).pipe(Effect.asVoid),
    close: Ref.update(closeCount, (count) => count + 1),
  };

  return {
    process,
    takeWrite: Queue.take(written),
    emitStdout: (text: string) => Queue.offer(stdout, textEncoder.encode(text)).pipe(Effect.asVoid),
    emitStderr: (text: string) => Queue.offer(stderr, textEncoder.encode(text)).pipe(Effect.asVoid),
    stderrObserved: Queue.take(stderrObserved),
    exit: (code: number) => Deferred.succeed(exitCode, code),
    closeCount: Ref.get(closeCount),
  };
});

const spawnWireCapture = () =>
  spawnSupervisedClient({
    command: "node",
    args: [
      "--input-type=commonjs",
      "--eval",
      `process.stdin.setEncoding("utf8");
let buffer = "";
let captured = false;
process.stdin.on("data", (chunk) => {
  buffer += chunk;
  const newline = buffer.indexOf("\\n");
  if (captured || newline < 0) return;
  captured = true;
  process.stdout.write(
    JSON.stringify({ type: "wire.capture", record: buffer.slice(0, newline + 1) }) + "\\n",
  );
});`,
    ],
    cwd: process.cwd(),
  });

describe("PrimeRpcClient", () => {
  it.effect("runs JSONL framing and ordered replay over an injected host process", () =>
    Effect.gen(function* () {
      const fake = yield* makeFakeProcess();
      const client = yield* makePrimeRpcClient(fake.process);
      const requestFiber = yield* client
        .request({ type: "prompt", message: "from host" })
        .pipe(Effect.forkChild);

      const record = new TextDecoder().decode(yield* fake.takeWrite);
      expect(record.endsWith("\n")).toBe(true);
      const command = decodeTestCommand(record);
      expect(command.type).toBe("prompt");

      yield* fake.emitStdout(
        `${encodeTestJson({ type: "first" })}\n${encodeTestJson({ type: "second" })}\n${encodeTestJson(
          {
            id: command.id,
            type: "response",
            command: "prompt",
            success: true,
          },
        )}\n`,
      );

      const response = yield* Fiber.join(requestFiber);
      expect(response.id).toBe(command.id);
      const replayed = yield* client.events.pipe(Stream.take(2), Stream.runCollect);
      expect(Array.from(replayed).map(eventType)).toEqual(["first", "second"]);
    }),
  );

  it.effect("fails a pending injected-process request with the stderr tail on exit", () =>
    Effect.gen(function* () {
      const fake = yield* makeFakeProcess();
      const client = yield* makePrimeRpcClient(fake.process);
      const resultFiber = yield* client
        .request({ type: "prompt", message: "pending" }, { timeoutMs: null })
        .pipe(Effect.result, Effect.forkChild);
      yield* fake.takeWrite;
      yield* fake.emitStderr("prime host failed");
      yield* fake.stderrObserved;
      // Pulling the next chunk proves the preceding chunk reached the client's sink.
      yield* fake.emitStderr("");
      yield* fake.stderrObserved;
      yield* fake.exit(17);

      const result = yield* Fiber.join(resultFiber);
      expect(result._tag).toBe("Failure");
      if (result._tag !== "Failure") {
        return;
      }
      expect(isPrimeRpcError(result.failure)).toBe(true);
      if (isPrimeRpcError(result.failure)) {
        expect(result.failure.operation).toBe("process");
        expect(result.failure.detail).toContain("Process exited with code 17.");
        expect(result.failure.detail).toContain("prime host failed");
      }
    }),
  );

  it.effect("delegates close to the injected host process once, including finalization", () =>
    Effect.gen(function* () {
      const fake = yield* makeFakeProcess();
      yield* Effect.scoped(
        Effect.gen(function* () {
          const client = yield* makePrimeRpcClient(fake.process);
          yield* client.close;
          yield* client.close;
        }),
      );

      expect(yield* fake.closeCount).toBe(1);
    }),
  );

  it.effect("maps an injected host write failure to the RPC operation", () =>
    Effect.gen(function* () {
      const fake = yield* makeFakeProcess();
      const client = yield* makePrimeRpcClient({
        ...fake.process,
        write: () =>
          Effect.fail(
            new ProviderAdapterHostProcessError({
              operation: "stdin",
              detail: "host rejected the write",
            }),
          ),
      });

      const result = yield* client
        .request({ type: "prompt", message: "write failure" })
        .pipe(Effect.result);
      expect(result._tag).toBe("Failure");
      if (result._tag !== "Failure") {
        return;
      }
      expect(isPrimeRpcError(result.failure)).toBe(true);
      if (isPrimeRpcError(result.failure)) {
        expect(result.failure.operation).toBe("prompt");
        expect(result.failure.detail).toBe("Failed to write command.");
      }
    }),
  );
  it.live("sends an extension UI response verbatim without waiting for a response", () =>
    provide(
      Effect.gen(function* () {
        const client = yield* spawnWireCapture();
        const captured = yield* Deferred.make<string>();
        yield* client.events.pipe(
          Stream.runForEach((event) => {
            if (
              !isRecord(event) ||
              event.type !== "wire.capture" ||
              typeof event.record !== "string"
            ) {
              return Effect.void;
            }
            return Deferred.succeed(captured, event.record);
          }),
          Effect.forkChild,
        );

        const result = yield* client.respondToExtensionUi({
          id: "prime-dialog-42",
          confirmed: false,
        });

        expect(result).toBeUndefined();
        expect(yield* Deferred.await(captured).pipe(Effect.timeout("2 seconds"))).toBe(
          '{"type":"extension_ui_response","id":"prime-dialog-42","confirmed":false}\n',
        );
      }),
    ),
  );

  it.live("fails an extension UI response that is not JSON-serializable", () =>
    provide(
      Effect.gen(function* () {
        const client = yield* spawnWireCapture();
        const result = yield* client
          .respondToExtensionUi({
            id: "prime-dialog-unserializable",
            value: 1n as unknown as string,
          })
          .pipe(Effect.result);

        expect(result._tag).toBe("Failure");
        if (result._tag !== "Failure") {
          return;
        }
        expect(isPrimeRpcError(result.failure)).toBe(true);
        if (isPrimeRpcError(result.failure)) {
          expect(result.failure.operation).toBe("extension_ui_response");
          expect(result.failure.detail).toBe("Response is not JSON-serializable.");
        }
      }),
    ),
  );

  it.live("fails an extension UI response after the RPC client closes", () =>
    provide(
      Effect.gen(function* () {
        const client = yield* spawnWireCapture();
        yield* client.close;

        const result = yield* client
          .respondToExtensionUi({ id: "prime-dialog-closed", cancelled: true })
          .pipe(Effect.result);

        expect(result._tag).toBe("Failure");
        if (result._tag !== "Failure") {
          return;
        }
        expect(isPrimeRpcError(result.failure)).toBe(true);
        if (isPrimeRpcError(result.failure)) {
          expect(result.failure.operation).toBe("extension_ui_response");
          expect(result.failure.detail).toBe("RPC process is closed.");
        }
      }),
    ),
  );

  it.live(
    "correlates a prompt response by request id and waits for agent_end on the event stream",
    () =>
      provide(
        Effect.gen(function* () {
          const client = yield* spawnMock();
          const agentEnd = yield* Deferred.make<unknown>();
          const events = yield* Ref.make<Array<unknown>>([]);
          yield* client.events.pipe(
            Stream.runForEach((event) =>
              Effect.gen(function* () {
                yield* Ref.update(events, (current) => [...current, event]);
                if (eventType(event) === "agent_end") {
                  yield* Deferred.succeed(agentEnd, event);
                }
              }),
            ),
            Effect.forkChild,
          );

          const response = yield* client.request({ type: "prompt", message: "hello" });

          expect(response.type).toBe("response");
          expect(response.command).toBe("prompt");
          expect(response.success).toBe(true);
          expect(response.id.startsWith("t3-")).toBe(true);
          expect(eventType(response)).not.toBe("agent_end");

          const end = yield* Deferred.await(agentEnd);
          expect(eventType(end)).toBe("agent_end");

          const collected = yield* Ref.get(events);
          expect(collected.map(eventType)).toEqual(["agent_start", "message_update", "agent_end"]);
        }),
      ),
  );

  it.live("fans events out to every subscriber", () =>
    provide(
      Effect.gen(function* () {
        const client = yield* spawnMock();
        const firstEnd = yield* Deferred.make<void>();
        const secondEnd = yield* Deferred.make<void>();
        const first = yield* Ref.make<Array<string>>([]);
        const second = yield* Ref.make<Array<string>>([]);

        const collect = (
          sink: Ref.Ref<Array<string>>,
          done: Deferred.Deferred<void>,
        ): Effect.Effect<void> =>
          client.events.pipe(
            Stream.runForEach((event) =>
              Effect.gen(function* () {
                const type = eventType(event);
                if (type === undefined) {
                  return;
                }
                yield* Ref.update(sink, (current) => [...current, type]);
                if (type === "agent_end") {
                  yield* Deferred.succeed(done, undefined);
                }
              }),
            ),
          );

        yield* collect(first, firstEnd).pipe(Effect.forkChild);
        yield* collect(second, secondEnd).pipe(Effect.forkChild);

        yield* client.request({ type: "prompt", message: "fanout" });
        yield* Deferred.await(firstEnd);
        yield* Deferred.await(secondEnd);

        expect(yield* Ref.get(first)).toEqual(["agent_start", "message_update", "agent_end"]);
        expect(yield* Ref.get(second)).toEqual(["agent_start", "message_update", "agent_end"]);
      }),
    ),
  );

  it.live("aborts a streaming prompt and still receives agent_end", () =>
    provide(
      Effect.gen(function* () {
        const client = yield* spawnMock();
        const ended = yield* Deferred.make<unknown>();
        yield* client.events.pipe(
          Stream.runForEach((event) =>
            eventType(event) === "agent_end" ? Deferred.succeed(ended, event) : Effect.void,
          ),
          Effect.forkChild,
        );

        yield* client.request({ type: "prompt", message: "slow turn" });
        const abort = yield* client.request({ type: "abort" });
        expect(abort.success).toBe(true);

        const end = yield* Deferred.await(ended);
        expect(eventType(end)).toBe("agent_end");
      }),
    ),
  );

  it.live("closes stdin and waits for the mock process to exit", () =>
    provide(
      Effect.gen(function* () {
        const fileSystem = yield* FileSystem.FileSystem;
        const tempDir = yield* fileSystem.makeTempDirectoryScoped({
          prefix: "t3-prime-rpc-exit-",
        });
        const exitLogPath = NodePath.join(tempDir, "exit.log");

        const log = yield* Effect.scoped(
          Effect.gen(function* () {
            const client = yield* spawnMock({
              T3_PRIME_MOCK_EXIT_LOG_PATH: exitLogPath,
            });
            yield* client.close;
            return yield* fileSystem.readFileString(exitLogPath);
          }),
        );

        expect(log).toContain("stdin-eof");
        expect(log).not.toContain("SIGTERM");
      }),
    ),
  );

  it.live("isolates split UTF-8 decoding across concurrent RPC clients", () =>
    provide(
      Effect.gen(function* () {
        const blocked = yield* spawnMock();
        yield* blocked.request({ type: "prompt", message: "hold split unicode" });

        const active = yield* spawnMock();
        const delta = yield* Deferred.make<string>();
        yield* active.events.pipe(
          Stream.runForEach((event) => {
            if (eventType(event) !== "message_update" || !isRecord(event)) {
              return Effect.void;
            }
            const update = event.assistantMessageEvent;
            return isRecord(update) && typeof update.delta === "string"
              ? Deferred.succeed(delta, update.delta)
              : Effect.void;
          }),
          Effect.forkChild,
        );

        yield* active.request({ type: "prompt", message: "emit line separator" });
        expect(yield* Deferred.await(delta)).toBe(`hello${LINE_SEPARATOR}world`);
      }),
    ),
  );

  it.live("keeps a JSON string containing U+2028 as one RPC record", () =>
    provide(
      Effect.gen(function* () {
        const client = yield* spawnMock();
        const delta = yield* Deferred.make<string>();
        yield* client.events.pipe(
          Stream.runForEach((event) =>
            Effect.gen(function* () {
              if (eventType(event) !== "message_update" || !isRecord(event)) {
                return;
              }
              const assistantMessageEvent = event.assistantMessageEvent;
              if (
                !isRecord(assistantMessageEvent) ||
                typeof assistantMessageEvent.delta !== "string"
              ) {
                return;
              }
              yield* Deferred.succeed(delta, assistantMessageEvent.delta);
            }),
          ),
          Effect.forkChild,
        );

        yield* client.request({ type: "prompt", message: "emit line separator" });
        const text = yield* Deferred.await(delta);
        expect(text).toBe(`hello${LINE_SEPARATOR}world`);
        expect(text).toContain(LINE_SEPARATOR);
        expect(text.split("\n")).toHaveLength(1);
      }),
    ),
  );
});
