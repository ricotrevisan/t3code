import {
  ApprovalRequestId,
  ProviderAdapterManifestV1,
  ProviderDriverKind,
  ProviderInstanceId,
  ThreadId,
  type ProviderRuntimeEvent,
} from "@t3tools/contracts";
import {
  ProviderAdapterHostProcessError,
  type ProviderAdapterHostV1,
  type ProviderAdapterProcessSpawnV1,
  type ProviderAdapterProcessV1,
} from "@t3tools/provider-adapter";
import { assert, describe, it } from "@effect/vitest";
import * as Cause from "effect/Cause";
import * as Deferred from "effect/Deferred";
import * as Effect from "effect/Effect";
import * as Exit from "effect/Exit";
import * as Option from "effect/Option";
import * as Fiber from "effect/Fiber";
import * as Queue from "effect/Queue";
import * as Ref from "effect/Ref";
import * as Schema from "effect/Schema";
import * as Stream from "effect/Stream";
import type * as AcpSchema from "effect-acp/schema";

import { defineAcpStdioAdapterV1, type AcpStdioAdapterConfig } from "./AcpStdioAdapter.ts";

interface WireMessage {
  readonly jsonrpc?: "2.0";
  readonly id?: string | number | null;
  readonly method?: string;
  readonly params?: unknown;
  readonly result?: unknown;
  readonly error?: unknown;
}

interface EffectGate {
  readonly entered: Deferred.Deferred<void>;
  readonly release: Deferred.Deferred<void>;
}

interface HarnessOptions {
  readonly configOptions?: ReadonlyArray<AcpSchema.SessionConfigOption>;
  readonly promptUpdates?: ReadonlyArray<AcpSchema.SessionNotification["update"]>;
  readonly permission?: AcpSchema.RequestPermissionRequest;
  readonly holdPromptUntilCancel?: boolean;
  readonly supportsResume?: boolean;
  readonly supportsClose?: boolean;
  readonly newSessionGate?: EffectGate;
  readonly configGate?: EffectGate;
  readonly promptGate?: EffectGate;
  readonly closeGate?: EffectGate;
  readonly cancelFails?: boolean;
  readonly closeFails?: boolean;
  readonly detachDefects?: boolean;
}

interface FakeProcessControl {
  readonly process: ProviderAdapterProcessV1;
  readonly spawnInput: ProviderAdapterProcessSpawnV1;
  readonly methods: Ref.Ref<Array<{ readonly method: string; readonly params: unknown }>>;
  readonly attached: Ref.Ref<Array<ThreadId>>;
  readonly detached: Ref.Ref<Array<ThreadId>>;
  readonly expectExitCount: Ref.Ref<number>;
  readonly closeCount: Ref.Ref<number>;
  readonly permissionResponse: Deferred.Deferred<unknown>;
  readonly promptSeen: Deferred.Deferred<void>;
  readonly cancelSeen: Deferred.Deferred<void>;
  readonly exit: (code?: number) => Effect.Effect<void>;
}

const encoder = new TextEncoder();
const decoder = new TextDecoder();
const permissionRequestId = 9001;

const encode = (message: WireMessage): Uint8Array =>
  encoder.encode(`${JSON.stringify({ jsonrpc: "2.0", ...message })}\n`);

const decode = (chunk: Uint8Array): WireMessage => JSON.parse(decoder.decode(chunk)) as WireMessage;

const modelOptions = (
  model = "model-a",
  reasoningValues: ReadonlyArray<string> = ["low"],
  reasoning = reasoningValues[0] ?? "low",
): ReadonlyArray<AcpSchema.SessionConfigOption> => [
  {
    type: "select",
    id: "model",
    name: "Model",
    currentValue: model,
    options: [
      { value: "model-a", name: "Model A" },
      { value: "model-b", name: "Model B" },
    ],
  },
  {
    type: "select",
    id: "reasoning_effort",
    name: "Reasoning",
    category: "thought_level",
    currentValue: reasoning,
    options: reasoningValues.map((value) => ({ value, name: value.toUpperCase() })),
  },
];

const makeHarness = Effect.fn("makeHarness")(function* (options: HarnessOptions = {}) {
  const processes = yield* Ref.make<Array<FakeProcessControl>>([]);
  const spawnCount = yield* Ref.make(0);
  const sessionCounter = yield* Ref.make(0);

  const spawn = (spawnInput: ProviderAdapterProcessSpawnV1) =>
    Effect.gen(function* () {
      yield* Ref.update(spawnCount, (count) => count + 1);
      const stdout = yield* Queue.unbounded<Uint8Array, Cause.Done<void>>();
      const stderr = yield* Queue.unbounded<Uint8Array, Cause.Done<void>>();
      const writes = yield* Queue.unbounded<Uint8Array, Cause.Done<void>>();
      const exitCode = yield* Deferred.make<number>();
      const exited = yield* Ref.make(false);
      const methods = yield* Ref.make<Array<{ readonly method: string; readonly params: unknown }>>(
        [],
      );
      const attached = yield* Ref.make<Array<ThreadId>>([]);
      const detached = yield* Ref.make<Array<ThreadId>>([]);
      const expectExitCount = yield* Ref.make(0);
      const closeCount = yield* Ref.make(0);
      const permissionResponse = yield* Deferred.make<unknown>();
      const promptSeen = yield* Deferred.make<void>();
      const cancelSeen = yield* Deferred.make<void>();
      const pendingPrompts = new Map<string | number | null, string>();

      const exit = (code = 0) =>
        Ref.getAndSet(exited, true).pipe(
          Effect.flatMap((alreadyExited) =>
            alreadyExited
              ? Effect.void
              : Deferred.succeed(exitCode, code).pipe(
                  Effect.andThen(Queue.end(stdout)),
                  Effect.andThen(Queue.end(stderr)),
                  Effect.andThen(Queue.end(writes)),
                  Effect.asVoid,
                ),
          ),
        );

      const respond = (id: WireMessage["id"], result: unknown) =>
        Queue.offer(stdout, encode({ ...(id === undefined ? {} : { id }), result })).pipe(
          Effect.asVoid,
        );
      const notify = (method: string, params: unknown) =>
        Queue.offer(stdout, encode({ method, params })).pipe(Effect.asVoid);
      const respondError = (id: WireMessage["id"], message: string) =>
        Queue.offer(
          stdout,
          encode({ ...(id === undefined ? {} : { id }), error: { code: -32_603, message } }),
        ).pipe(Effect.asVoid);

      const handleRequest = Effect.fn("fakeAcp.handleRequest")(function* (message: WireMessage) {
        if (message.method) {
          yield* Ref.update(methods, (entries) => [
            ...entries,
            { method: message.method!, params: message.params },
          ]);
        }

        switch (message.method) {
          case "initialize":
            yield* respond(message.id, {
              protocolVersion: 1,
              agentInfo: { name: "fake-acp", version: "1.0.0" },
              agentCapabilities: {
                loadSession: false,
                sessionCapabilities: {
                  ...(options.supportsResume === false ? {} : { resume: {} }),
                  ...(options.supportsClose === false ? {} : { close: {} }),
                },
              },
            });
            return;
          case "session/new": {
            const next = yield* Ref.modify(sessionCounter, (current) => [current + 1, current + 1]);
            const result = {
              sessionId: `session-${next}`,
              configOptions: options.configOptions ?? modelOptions(),
            };
            if (options.newSessionGate) {
              yield* Deferred.succeed(options.newSessionGate.entered, undefined).pipe(
                Effect.ignore,
              );
              yield* Deferred.await(options.newSessionGate.release).pipe(
                Effect.andThen(respond(message.id, result)),
                Effect.forkScoped,
              );
            } else {
              yield* respond(message.id, result);
            }
            return;
          }
          case "session/resume":
            yield* respond(message.id, {
              configOptions: options.configOptions ?? modelOptions(),
            });
            return;
          case "session/set_config_option": {
            const params = message.params as {
              readonly sessionId: string;
              readonly configId: string;
              readonly value: string;
            };
            const configOptions =
              params.configId === "model"
                ? modelOptions(params.value, ["low", "high"], "low")
                : modelOptions("model-b", ["low", "high"], params.value);
            if (options.configGate) {
              yield* Deferred.succeed(options.configGate.entered, undefined).pipe(Effect.ignore);
              yield* Deferred.await(options.configGate.release).pipe(
                Effect.andThen(respond(message.id, { configOptions })),
                Effect.forkScoped,
              );
            } else {
              yield* respond(message.id, { configOptions });
            }
            return;
          }
          case "session/prompt": {
            yield* Deferred.succeed(promptSeen, undefined).pipe(Effect.ignore);
            const params = message.params as { readonly sessionId: string };
            pendingPrompts.set(message.id ?? null, params.sessionId);
            for (const update of options.promptUpdates ?? []) {
              yield* notify("session/update", { sessionId: params.sessionId, update });
            }
            if (options.permission) {
              yield* Queue.offer(
                stdout,
                encode({
                  id: permissionRequestId,
                  method: "session/request_permission",
                  params: { ...options.permission, sessionId: params.sessionId },
                }),
              );
              return;
            }
            if (options.promptGate) {
              yield* Deferred.succeed(options.promptGate.entered, undefined).pipe(Effect.ignore);
              yield* Deferred.await(options.promptGate.release).pipe(
                Effect.andThen(Effect.sync(() => pendingPrompts.delete(message.id ?? null))),
                Effect.andThen(respond(message.id, { stopReason: "end_turn" })),
                Effect.forkScoped,
              );
            } else if (!options.holdPromptUntilCancel) {
              pendingPrompts.delete(message.id ?? null);
              yield* respond(message.id, { stopReason: "end_turn" });
            }
            return;
          }
          case "session/cancel":
            yield* Deferred.succeed(cancelSeen, undefined).pipe(Effect.ignore);
            if (options.cancelFails) {
              for (const [id] of pendingPrompts) {
                pendingPrompts.delete(id);
                yield* respond(id, { stopReason: "cancelled" });
              }
              return;
            }
            for (const [id] of pendingPrompts) {
              pendingPrompts.delete(id);
              yield* respond(id, { stopReason: "cancelled" });
            }
            return;
          case "session/close":
            if (options.closeFails) {
              yield* respondError(message.id, "close failed");
            } else if (options.closeGate) {
              yield* Deferred.succeed(options.closeGate.entered, undefined).pipe(Effect.ignore);
              yield* Deferred.await(options.closeGate.release).pipe(
                Effect.andThen(respond(message.id, {})),
                Effect.forkScoped,
              );
            } else {
              yield* respond(message.id, {});
            }
            return;
        }

        if (message.id === permissionRequestId && message.result !== undefined) {
          yield* Deferred.succeed(permissionResponse, message.result).pipe(Effect.ignore);
          for (const [id] of pendingPrompts) {
            pendingPrompts.delete(id);
            yield* respond(id, { stopReason: "end_turn" });
          }
        }
      });

      yield* Stream.fromQueue(writes).pipe(
        Stream.runForEach((chunk) => handleRequest(decode(chunk))),
        Effect.forkScoped,
      );

      const process = {
        pid: 4000 + (yield* Ref.get(spawnCount)),
        attachSession: (threadId) => Ref.update(attached, (values) => [...values, threadId]),
        detachSession: (threadId) =>
          options.detachDefects
            ? Effect.die("detach defect")
            : Ref.update(detached, (values) => [...values, threadId]),
        expectExit: Ref.update(expectExitCount, (count) => count + 1),
        stdout: Stream.fromQueue(stdout),
        stderr: Stream.fromQueue(stderr),
        exitCode: Deferred.await(exitCode),
        write: (chunk) =>
          options.cancelFails && decode(chunk).method === "session/cancel"
            ? Effect.fail(
                new ProviderAdapterHostProcessError({
                  operation: "stdin",
                  detail: "cancel failed",
                }),
              )
            : Queue.offer(writes, chunk).pipe(Effect.asVoid),
        close: Ref.update(closeCount, (count) => count + 1).pipe(Effect.andThen(exit())),
      } satisfies ProviderAdapterProcessV1;

      const control: FakeProcessControl = {
        process,
        spawnInput,
        methods,
        attached,
        detached,
        expectExitCount,
        closeCount,
        permissionResponse,
        promptSeen,
        cancelSeen,
        exit,
      };
      yield* Ref.update(processes, (values) => [...values, control]);
      return process;
    });

  return {
    host: { protocolVersion: 1, processes: { spawn } } satisfies ProviderAdapterHostV1,
    processes,
    spawnCount,
  };
});

const manifest = Schema.decodeUnknownSync(ProviderAdapterManifestV1)({
  protocolVersion: 1,
  id: "test-acp-stdio",
  version: "1.0.0",
  driver: "testAcp",
  displayName: "Test ACP",
  hostProtocol: { minimum: 1, maximum: 1 },
  transport: {
    kind: "supervised-stdio",
    protocol: "acp-v1",
    sessionConcurrency: "multiplexed",
  },
  capabilities: [
    "session.resume",
    "turn.interrupt",
    "request.approval",
    "model.discovery",
    "model.switch",
    "reasoning.selection",
    "stream.reasoning",
    "stream.tool-lifecycle",
    "stream.usage",
    "stream.context",
    "input.attachments",
    "conversation.rollback",
    "request.structured-input",
  ],
  configSchema: { type: "object" },
});

const config: AcpStdioAdapterConfig = { command: "fake-acp", args: ["--stdio"] };
const instanceId = ProviderInstanceId.make("test-acp-instance");
const packageDefinition = defineAcpStdioAdapterV1({
  manifest,
  defaultConfig: () => config,
  clientInfo: { name: "t3-test", version: "0.0.0" },
});

const createInstance = (host: ProviderAdapterHostV1, definition = packageDefinition) =>
  definition.create(
    {
      instanceId,
      displayName: undefined,
      accentColor: undefined,
      environment: { TEST_FROM_HOST: "yes" },
      enabled: true,
      config,
    },
    host,
  );

const makeGate = Effect.fn("makeGate")(function* (): Effect.fn.Return<EffectGate> {
  return {
    entered: yield* Deferred.make<void>(),
    release: yield* Deferred.make<void>(),
  };
});

const manifestWithoutResume = Schema.decodeUnknownSync(ProviderAdapterManifestV1)({
  ...manifest,
  capabilities: manifest.capabilities.filter((feature) => feature !== "session.resume"),
});
const packageWithoutResume = defineAcpStdioAdapterV1({
  manifest: manifestWithoutResume,
  defaultConfig: () => config,
});
const incompatibleManifest = Schema.decodeUnknownSync(ProviderAdapterManifestV1)({
  ...manifest,
  transport: {
    kind: "supervised-stdio",
    protocol: "acp-v1",
    sessionConcurrency: "one-per-process",
  },
});
const incompatiblePackage = defineAcpStdioAdapterV1({
  manifest: incompatibleManifest,
  defaultConfig: () => config,
});

const startInput = (threadId: ThreadId, overrides: Record<string, unknown> = {}) => ({
  threadId,
  provider: ProviderDriverKind.make("testAcp"),
  providerInstanceId: instanceId,
  cwd: "/tmp/acp-test",
  runtimeMode: "approval-required" as const,
  ...overrides,
});

const methodsOf = (control: FakeProcessControl) =>
  Ref.get(control.methods).pipe(Effect.map((entries) => entries.map((entry) => entry.method)));

const containsRawKey = (value: unknown): boolean => {
  if (value === null || typeof value !== "object") return false;
  if (Array.isArray(value)) return value.some(containsRawKey);
  return (
    Object.prototype.hasOwnProperty.call(value, "raw") || Object.values(value).some(containsRawKey)
  );
};

describe("AcpStdioAdapter", () => {
  it.effect("multiplexes new and resumed sessions on one host-owned process", () =>
    Effect.scoped(
      Effect.gen(function* () {
        const harness = yield* makeHarness();
        const instance = yield* createInstance(harness.host);
        const threadA = ThreadId.make("thread-a");
        const threadB = ThreadId.make("thread-b");

        const fresh = yield* instance.adapter.startSession(startInput(threadA));
        const resumed = yield* instance.adapter.startSession(
          startInput(threadB, {
            resumeCursor: { kind: "acp-v1", protocolVersion: 1, sessionId: "saved-session" },
          }),
        );

        assert.equal(yield* Ref.get(harness.spawnCount), 1);
        const [control] = yield* Ref.get(harness.processes);
        assert.isDefined(control);
        assert.deepEqual(yield* methodsOf(control!), [
          "initialize",
          "session/new",
          "session/resume",
        ]);
        assert.deepEqual(yield* Ref.get(control!.attached), [threadB]);
        assert.equal((fresh.resumeCursor as { sessionId: string }).sessionId, "session-1");
        assert.equal((resumed.resumeCursor as { sessionId: string }).sessionId, "saved-session");
        assert.deepEqual(control!.spawnInput, {
          command: "fake-acp",
          args: ["--stdio"],
          environment: { TEST_FROM_HOST: "yes" },
          purpose: { kind: "session", threadId: threadA },
        });

        yield* instance.adapter.stopSession(threadA);
        yield* instance.adapter.stopSession(threadB);
        assert.deepEqual((yield* methodsOf(control!)).slice(-2), [
          "session/close",
          "session/close",
        ]);
        assert.deepEqual(yield* Ref.get(control!.detached), [threadA, threadB]);

        yield* instance.adapter.stopAll();
        assert.equal(yield* Ref.get(control!.expectExitCount), 1);
        assert.equal(yield* Ref.get(control!.closeCount), 1);
      }),
    ),
  );

  it.effect("applies the model before refreshed model-dependent reasoning", () =>
    Effect.scoped(
      Effect.gen(function* () {
        const harness = yield* makeHarness({ configOptions: modelOptions() });
        const instance = yield* createInstance(harness.host);
        const threadId = ThreadId.make("thread-config");

        yield* instance.adapter.startSession(
          startInput(threadId, {
            modelSelection: {
              instanceId,
              model: "model-b",
              options: [{ id: "reasoningEffort", value: "high" }],
            },
          }),
        );

        const [control] = yield* Ref.get(harness.processes);
        const calls = (yield* Ref.get(control!.methods)).filter(
          (entry) => entry.method === "session/set_config_option",
        );
        assert.deepEqual(
          calls.map((entry) => entry.params),
          [
            { sessionId: "session-1", configId: "model", value: "model-b" },
            { sessionId: "session-1", configId: "reasoning_effort", value: "high" },
          ],
        );
        const snapshot = yield* instance.snapshot.getSnapshot;
        assert.deepEqual(
          snapshot.models.map((model) => [model.slug, model.isDefault]),
          [
            ["model-a", undefined],
            ["model-b", true],
          ],
        );
      }),
    ),
  );

  it.effect("maps message, thought, tool, and context usage updates in order", () =>
    Effect.scoped(
      Effect.gen(function* () {
        const harness = yield* makeHarness({
          promptUpdates: [
            {
              sessionUpdate: "agent_message_chunk",
              messageId: "assistant-1",
              content: { type: "text", text: "hello" },
            },
            {
              sessionUpdate: "agent_thought_chunk",
              messageId: "thought-1",
              content: { type: "text", text: "thinking" },
            },
            {
              sessionUpdate: "tool_call",
              toolCallId: "tool-1",
              title: "Run command",
              kind: "execute",
              status: "pending",
            },
            {
              sessionUpdate: "tool_call_update",
              toolCallId: "tool-1",
              status: "completed",
              rawOutput: { ok: true },
            },
            {
              sessionUpdate: "available_commands_update",
              availableCommands: [
                { name: "review", description: "Review changes", input: { hint: "focus" } },
              ],
            },
            { sessionUpdate: "usage_update", used: 12, size: 100 },
          ],
        });
        const instance = yield* createInstance(harness.host);
        const threadId = ThreadId.make("thread-events");
        yield* instance.adapter.startSession(startInput(threadId));

        const collected = yield* Ref.make<Array<ProviderRuntimeEvent>>([]);
        const completed = yield* Deferred.make<void>();
        yield* instance.adapter.streamEvents.pipe(
          Stream.runForEach((event) =>
            Ref.update(collected, (events) => [...events, event]).pipe(
              Effect.andThen(
                event.type === "turn.completed"
                  ? Deferred.succeed(completed, undefined).pipe(Effect.asVoid)
                  : Effect.void,
              ),
            ),
          ),
          Effect.forkScoped,
        );
        yield* Effect.yieldNow;
        yield* instance.adapter.sendTurn({ threadId, input: "go" });
        yield* Deferred.await(completed);
        const events = yield* Ref.get(collected);

        assert.deepEqual(
          events.map((event) => event.type),
          [
            "turn.started",
            "item.started",
            "content.delta",
            "item.started",
            "content.delta",
            "item.started",
            "item.completed",
            "session.configured",
            "thread.token-usage.updated",
            "item.completed",
            "item.completed",
            "turn.completed",
          ],
        );
        const usage = events.find((event) => event.type === "thread.token-usage.updated");
        assert.deepEqual(usage?.payload, { usage: { usedTokens: 12, maxTokens: 100 } });
        assert.deepEqual((yield* instance.snapshot.getSnapshot).slashCommands, [
          { name: "review", description: "Review changes", input: { hint: "focus" } },
        ]);
        const snapshot = yield* instance.adapter.readThread(threadId);
        assert.equal(snapshot.turns.length, 1);
        assert.equal(snapshot.turns[0]?.items.length, 13);
      }),
    ),
  );

  it.effect("correlates permission decisions to the matching ACP option id", () =>
    Effect.scoped(
      Effect.gen(function* () {
        const harness = yield* makeHarness({
          permission: {
            sessionId: "replaced-by-harness",
            options: [
              { optionId: "allow-this-time", name: "Allow", kind: "allow_once" },
              { optionId: "never", name: "Reject", kind: "reject_once" },
            ],
            toolCall: {
              toolCallId: "tool-permission",
              title: "Execute tests",
              kind: "execute",
              status: "pending",
            },
          },
        });
        const instance = yield* createInstance(harness.host);
        const threadId = ThreadId.make("thread-permission");
        yield* instance.adapter.startSession(startInput(threadId));

        const openedFiber = yield* instance.adapter.streamEvents.pipe(
          Stream.filter((event) => event.type === "request.opened"),
          Stream.runHead,
          Effect.forkScoped,
        );
        const completedFiber = yield* instance.adapter.streamEvents.pipe(
          Stream.filter((event) => event.type === "turn.completed"),
          Stream.runHead,
          Effect.forkScoped,
        );
        yield* Effect.yieldNow;
        yield* instance.adapter.sendTurn({ threadId, input: "run" });
        const opened = Option.getOrThrow(yield* Fiber.join(openedFiber));
        const requestId = ApprovalRequestId.make(opened.requestId!);
        yield* instance.adapter.respondToRequest(threadId, requestId, "accept");

        const [control] = yield* Ref.get(harness.processes);
        assert.deepEqual(yield* Deferred.await(control!.permissionResponse), {
          outcome: { outcome: "selected", optionId: "allow-this-time" },
        });
        const completed = Option.getOrThrow(yield* Fiber.join(completedFiber));
        assert.deepEqual(completed.payload, {
          state: "completed",
          stopReason: "end_turn",
        });
      }),
    ),
  );

  it.effect("cancels an active prompt and records a cancelled turn", () =>
    Effect.scoped(
      Effect.gen(function* () {
        const harness = yield* makeHarness({ holdPromptUntilCancel: true });
        const instance = yield* createInstance(harness.host);
        const threadId = ThreadId.make("thread-cancel");
        yield* instance.adapter.startSession(startInput(threadId));
        const completedFiber = yield* instance.adapter.streamEvents.pipe(
          Stream.filter((event) => event.type === "turn.completed"),
          Stream.runHead,
          Effect.forkScoped,
        );
        yield* Effect.yieldNow;
        const turn = yield* instance.adapter.sendTurn({ threadId, input: "wait" });
        const [control] = yield* Ref.get(harness.processes);
        yield* Deferred.await(control!.promptSeen);
        yield* instance.adapter.interruptTurn(threadId, turn.turnId);

        yield* Deferred.await(control!.cancelSeen);
        const completed = Option.getOrThrow(yield* Fiber.join(completedFiber));
        assert.deepEqual(completed.payload, {
          state: "cancelled",
          stopReason: "cancelled",
        });
      }),
    ),
  );

  it.effect("stops an active session with cancel then close and one terminal event", () =>
    Effect.scoped(
      Effect.gen(function* () {
        const harness = yield* makeHarness({ holdPromptUntilCancel: true });
        const instance = yield* createInstance(harness.host);
        const threadId = ThreadId.make("thread-stop-active");
        yield* instance.adapter.startSession(startInput(threadId));
        const terminals = yield* Ref.make<Array<ProviderRuntimeEvent>>([]);
        const terminalObserved = yield* Deferred.make<void>();
        yield* instance.adapter.streamEvents.pipe(
          Stream.filter((event) => event.type === "turn.completed"),
          Stream.runForEach((event) =>
            Ref.update(terminals, (events) => [...events, event]).pipe(
              Effect.andThen(Deferred.succeed(terminalObserved, undefined)),
              Effect.asVoid,
            ),
          ),
          Effect.forkScoped,
        );
        yield* Effect.yieldNow;

        yield* instance.adapter.sendTurn({ threadId, input: "wait then stop" });
        const [control] = yield* Ref.get(harness.processes);
        yield* Deferred.await(control!.promptSeen);
        yield* instance.adapter.stopSession(threadId);
        assert.isTrue(Option.isSome(yield* Deferred.poll(terminalObserved)));

        const methods = yield* methodsOf(control!);
        assert.isBelow(methods.indexOf("session/cancel"), methods.indexOf("session/close"));
        const completed = yield* Ref.get(terminals);
        assert.equal(completed.length, 1);
        assert.deepEqual(completed[0]?.payload, {
          state: "cancelled",
          stopReason: "session stopped",
        });
      }),
    ),
  );

  it.effect("clears exited sessions and lazily recovers with a new process", () =>
    Effect.scoped(
      Effect.gen(function* () {
        const harness = yield* makeHarness({ holdPromptUntilCancel: true });
        const instance = yield* createInstance(harness.host);
        const threadId = ThreadId.make("thread-recovery");
        yield* instance.adapter.startSession(startInput(threadId));
        const snapshotChange = yield* instance.snapshot.streamChanges.pipe(
          Stream.runHead,
          Effect.forkScoped,
        );
        const terminal = yield* instance.adapter.streamEvents.pipe(
          Stream.filter((event) => event.type === "turn.completed"),
          Stream.runHead,
          Effect.forkScoped,
        );
        yield* Effect.yieldNow;

        yield* instance.adapter.sendTurn({ threadId, input: "exit during turn" });
        const [first] = yield* Ref.get(harness.processes);
        yield* Deferred.await(first!.promptSeen);
        yield* first!.exit(17);
        yield* Fiber.join(snapshotChange);
        const failed = Option.getOrThrow(yield* Fiber.join(terminal));
        assert.deepEqual(failed.payload, {
          state: "failed",
          errorMessage: "ACP process exited unexpectedly.",
        });
        assert.isFalse(yield* instance.adapter.hasSession(threadId));

        yield* instance.adapter.startSession(
          startInput(threadId, {
            resumeCursor: { kind: "acp-v1", protocolVersion: 1, sessionId: "session-1" },
          }),
        );
        assert.equal(yield* Ref.get(harness.spawnCount), 2);
        assert.isTrue(yield* instance.adapter.hasSession(threadId));
      }),
    ),
  );

  it.effect("does not let an old process exit clear a replacement process session", () =>
    Effect.scoped(
      Effect.gen(function* () {
        const harness = yield* makeHarness();
        const instance = yield* createInstance(harness.host);
        const oldThread = ThreadId.make("thread-old-process");
        const newThread = ThreadId.make("thread-new-process");
        yield* instance.adapter.startSession(startInput(oldThread));
        yield* instance.adapter.stopAll();
        yield* instance.adapter.startSession(startInput(newThread));
        yield* Effect.yieldNow;

        assert.equal(yield* Ref.get(harness.spawnCount), 2);
        assert.isTrue(yield* instance.adapter.hasSession(newThread));
      }),
    ),
  );

  it.effect("serializes concurrent starts and reserves a thread before session/new", () =>
    Effect.scoped(
      Effect.gen(function* () {
        const gate = yield* makeGate();
        const harness = yield* makeHarness({ newSessionGate: gate });
        const instance = yield* createInstance(harness.host);
        const threadId = ThreadId.make("thread-concurrent-start");
        const first = yield* instance.adapter
          .startSession(startInput(threadId))
          .pipe(Effect.result, Effect.forkScoped);
        yield* Deferred.await(gate.entered);
        const second = yield* instance.adapter
          .startSession(startInput(threadId))
          .pipe(Effect.result, Effect.forkScoped);

        const [control] = yield* Ref.get(harness.processes);
        assert.equal(
          (yield* methodsOf(control!)).filter((method) => method === "session/new").length,
          1,
        );
        yield* Deferred.succeed(gate.release, undefined);
        const results = [yield* Fiber.join(first), yield* Fiber.join(second)];
        assert.equal(results.filter((result) => result._tag === "Success").length, 1);
        assert.equal(results.filter((result) => result._tag === "Failure").length, 1);
      }),
    ),
  );

  it.effect("reserves a turn before delayed config so concurrent sends cannot both start", () =>
    Effect.scoped(
      Effect.gen(function* () {
        const gate = yield* makeGate();
        const harness = yield* makeHarness({ configGate: gate, holdPromptUntilCancel: true });
        const instance = yield* createInstance(harness.host);
        const threadId = ThreadId.make("thread-concurrent-send");
        yield* instance.adapter.startSession(startInput(threadId));
        const selected = {
          instanceId,
          model: "model-b",
          options: [{ id: "reasoningEffort", value: "high" }],
        };
        const first = yield* instance.adapter
          .sendTurn({ threadId, input: "first", modelSelection: selected })
          .pipe(Effect.result, Effect.forkScoped);
        yield* Deferred.await(gate.entered);
        const second = yield* instance.adapter
          .sendTurn({ threadId, input: "second", modelSelection: selected })
          .pipe(Effect.result, Effect.forkScoped);
        yield* Deferred.succeed(gate.release, undefined);

        const results = [yield* Fiber.join(first), yield* Fiber.join(second)];
        assert.equal(results.filter((result) => result._tag === "Success").length, 1);
        assert.equal(results.filter((result) => result._tag === "Failure").length, 1);
        const [control] = yield* Ref.get(harness.processes);
        yield* Deferred.await(control!.promptSeen);
        assert.equal(
          (yield* methodsOf(control!)).filter((method) => method === "session/prompt").length,
          1,
        );
        yield* instance.adapter.stopSession(threadId);
      }),
    ),
  );

  it.effect("serializes a delayed send against stop without orphaning the turn", () =>
    Effect.scoped(
      Effect.gen(function* () {
        const gate = yield* makeGate();
        const harness = yield* makeHarness({ configGate: gate, holdPromptUntilCancel: true });
        const instance = yield* createInstance(harness.host);
        const threadId = ThreadId.make("thread-send-stop-race");
        yield* instance.adapter.startSession(startInput(threadId));
        const terminals = yield* Ref.make<Array<ProviderRuntimeEvent>>([]);
        yield* instance.adapter.streamEvents.pipe(
          Stream.filter((event) => event.type === "turn.completed"),
          Stream.runForEach((event) => Ref.update(terminals, (items) => [...items, event])),
          Effect.forkScoped,
        );
        const sending = yield* instance.adapter
          .sendTurn({
            threadId,
            input: "configure then stop",
            modelSelection: { instanceId, model: "model-b" },
          })
          .pipe(Effect.result, Effect.forkScoped);
        yield* Deferred.await(gate.entered);
        const stopping = yield* instance.adapter
          .stopSession(threadId)
          .pipe(Effect.result, Effect.forkScoped);
        yield* Deferred.succeed(gate.release, undefined);
        assert.equal((yield* Fiber.join(sending))._tag, "Success");
        assert.equal((yield* Fiber.join(stopping))._tag, "Success");
        yield* Effect.yieldNow;
        assert.isFalse(yield* instance.adapter.hasSession(threadId));
        assert.equal((yield* Ref.get(terminals)).length, 1);
      }),
    ),
  );

  it.effect("rejects a duplicate resumed ACP session id without evicting its owner", () =>
    Effect.scoped(
      Effect.gen(function* () {
        const harness = yield* makeHarness();
        const instance = yield* createInstance(harness.host);
        const ownerThread = ThreadId.make("thread-session-owner");
        const duplicateThread = ThreadId.make("thread-session-duplicate");
        yield* instance.adapter.startSession(startInput(ownerThread));
        const duplicate = yield* instance.adapter
          .startSession(
            startInput(duplicateThread, {
              resumeCursor: { kind: "acp-v1", protocolVersion: 1, sessionId: "session-1" },
            }),
          )
          .pipe(Effect.result);

        assert.equal(duplicate._tag, "Failure");
        assert.isTrue(yield* instance.adapter.hasSession(ownerThread));
        assert.isFalse(yield* instance.adapter.hasSession(duplicateThread));
        const [control] = yield* Ref.get(harness.processes);
        assert.deepEqual(yield* Ref.get(control!.detached), [duplicateThread]);
      }),
    ),
  );

  it.effect("cleans local ownership even when cancel and close fail", () =>
    Effect.scoped(
      Effect.gen(function* () {
        const harness = yield* makeHarness({
          holdPromptUntilCancel: true,
          cancelFails: true,
          closeFails: true,
        });
        const instance = yield* createInstance(harness.host);
        const threadId = ThreadId.make("thread-stop-failure");
        yield* instance.adapter.startSession(startInput(threadId));
        yield* instance.adapter.sendTurn({ threadId, input: "active" });
        const [control] = yield* Ref.get(harness.processes);
        yield* Deferred.await(control!.promptSeen);

        const stopped = yield* instance.adapter.stopSession(threadId).pipe(Effect.result);
        assert.equal(stopped._tag, "Failure");
        assert.isFalse(yield* instance.adapter.hasSession(threadId));
        assert.deepEqual(yield* Ref.get(control!.detached), [threadId]);
      }),
    ),
  );

  it.effect("runs local stop cleanup when interrupted during session/close", () =>
    Effect.scoped(
      Effect.gen(function* () {
        const closeGate = yield* makeGate();
        const harness = yield* makeHarness({ closeGate });
        const instance = yield* createInstance(harness.host);
        const threadId = ThreadId.make("thread-stop-interrupted");
        yield* instance.adapter.startSession(startInput(threadId));
        const stopping = yield* instance.adapter.stopSession(threadId).pipe(Effect.forkScoped);
        yield* Deferred.await(closeGate.entered);
        yield* Fiber.interrupt(stopping);

        assert.isFalse(yield* instance.adapter.hasSession(threadId));
        const [control] = yield* Ref.get(harness.processes);
        assert.deepEqual(yield* Ref.get(control!.detached), [threadId]);
      }),
    ),
  );

  it.effect("keeps local stop cleanup complete when detach defects", () =>
    Effect.scoped(
      Effect.gen(function* () {
        const harness = yield* makeHarness({ detachDefects: true });
        const instance = yield* createInstance(harness.host);
        const threadId = ThreadId.make("thread-detach-defect");
        yield* instance.adapter.startSession(startInput(threadId));
        yield* instance.adapter.stopSession(threadId);

        assert.isFalse(yield* instance.adapter.hasSession(threadId));
        assert.deepEqual((yield* instance.snapshot.getSnapshot).models, []);
      }),
    ),
  );

  it.effect("claims a permission once across concurrent responses", () =>
    Effect.scoped(
      Effect.gen(function* () {
        const harness = yield* makeHarness({
          permission: {
            sessionId: "replaced",
            options: [{ optionId: "allow-once", name: "Allow", kind: "allow_once" }],
            toolCall: { toolCallId: "permission-race", title: "Run", kind: "execute" },
          },
        });
        const instance = yield* createInstance(harness.host);
        const threadId = ThreadId.make("thread-permission-race");
        yield* instance.adapter.startSession(startInput(threadId));
        const opened = yield* instance.adapter.streamEvents.pipe(
          Stream.filter((event) => event.type === "request.opened"),
          Stream.runHead,
          Effect.forkScoped,
        );
        const resolvedCount = yield* Ref.make(0);
        yield* instance.adapter.streamEvents.pipe(
          Stream.filter((event) => event.type === "request.resolved"),
          Stream.runForEach(() => Ref.update(resolvedCount, (count) => count + 1)),
          Effect.forkScoped,
        );
        yield* Effect.yieldNow;
        yield* instance.adapter.sendTurn({ threadId, input: "permission" });
        const request = Option.getOrThrow(yield* Fiber.join(opened));
        const requestId = ApprovalRequestId.make(request.requestId!);
        const first = yield* instance.adapter
          .respondToRequest(threadId, requestId, "accept")
          .pipe(Effect.result, Effect.forkScoped);
        const second = yield* instance.adapter
          .respondToRequest(threadId, requestId, "accept")
          .pipe(Effect.result, Effect.forkScoped);
        const results = [yield* Fiber.join(first), yield* Fiber.join(second)];
        assert.equal(results.filter((result) => result._tag === "Success").length, 1);
        assert.equal(results.filter((result) => result._tag === "Failure").length, 1);
        yield* Effect.yieldNow;
        assert.equal(yield* Ref.get(resolvedCount), 1);
      }),
    ),
  );

  it.effect("enforces the manifest resume promise in both directions", () =>
    Effect.scoped(
      Effect.gen(function* () {
        const missingAgentCapability = yield* makeHarness({ supportsResume: false });
        const required = yield* createInstance(missingAgentCapability.host);
        const requiredStart = yield* required.adapter
          .startSession(startInput(ThreadId.make("thread-resume-required")))
          .pipe(Effect.result);
        assert.equal(requiredStart._tag, "Failure");
        const [failedProcess] = yield* Ref.get(missingAgentCapability.processes);
        assert.equal(yield* Ref.get(failedProcess!.expectExitCount), 1);
        assert.equal(yield* Ref.get(failedProcess!.closeCount), 1);

        const undeclaredHarness = yield* makeHarness({ supportsResume: true });
        const undeclared = yield* createInstance(undeclaredHarness.host, packageWithoutResume);
        const resume = yield* undeclared.adapter
          .startSession(
            startInput(ThreadId.make("thread-resume-undeclared"), {
              resumeCursor: { kind: "acp-v1", protocolVersion: 1, sessionId: "saved" },
            }),
          )
          .pipe(Effect.result);
        assert.equal(resume._tag, "Failure");
        assert.notInclude(undeclared.adapter.capabilities.features, "session.resume");
        assert.equal(yield* Ref.get(undeclaredHarness.spawnCount), 0);
      }),
    ),
  );

  it.effect("restores snapshot authority after stopping the most recently configured session", () =>
    Effect.scoped(
      Effect.gen(function* () {
        const harness = yield* makeHarness();
        const instance = yield* createInstance(harness.host);
        const threadA = ThreadId.make("thread-snapshot-a");
        const threadB = ThreadId.make("thread-snapshot-b");
        yield* instance.adapter.startSession(startInput(threadA));
        yield* instance.adapter.startSession(
          startInput(threadB, {
            modelSelection: { instanceId, model: "model-b" },
          }),
        );
        assert.equal(
          (yield* instance.snapshot.getSnapshot).models.find((model) => model.isDefault)?.slug,
          "model-b",
        );
        yield* instance.adapter.stopSession(threadB);
        assert.equal(
          (yield* instance.snapshot.getSnapshot).models.find((model) => model.isDefault)?.slug,
          "model-a",
        );
      }),
    ),
  );

  it.effect("updates session and snapshot models from config notifications", () =>
    Effect.scoped(
      Effect.gen(function* () {
        const harness = yield* makeHarness({
          promptUpdates: [
            {
              sessionUpdate: "config_option_update",
              configOptions: modelOptions("model-b", ["low", "high"], "high"),
            },
          ],
        });
        const instance = yield* createInstance(harness.host);
        const threadId = ThreadId.make("thread-config-notification");
        yield* instance.adapter.startSession(startInput(threadId));
        const terminal = yield* instance.adapter.streamEvents.pipe(
          Stream.filter((event) => event.type === "turn.completed"),
          Stream.runHead,
          Effect.forkScoped,
        );
        yield* Effect.yieldNow;
        yield* instance.adapter.sendTurn({ threadId, input: "refresh config" });
        yield* Fiber.join(terminal);

        assert.equal((yield* instance.adapter.listSessions())[0]?.model, "model-b");
        assert.equal(
          (yield* instance.snapshot.getSnapshot).models.find((model) => model.isDefault)?.slug,
          "model-b",
        );
      }),
    ),
  );

  it.effect("bounds and sanitizes local readThread transcript retention", () =>
    Effect.scoped(
      Effect.gen(function* () {
        const harness = yield* makeHarness({
          promptUpdates: Array.from({ length: 220 }, (_, index) => ({
            sessionUpdate: "agent_message_chunk" as const,
            messageId: "bounded-message",
            content: { type: "text" as const, text: `chunk-${index}` },
          })),
        });
        const instance = yield* createInstance(harness.host);
        const threadId = ThreadId.make("thread-bounded-transcript");
        yield* instance.adapter.startSession(startInput(threadId));
        const terminal = yield* instance.adapter.streamEvents.pipe(
          Stream.filter((event) => event.type === "turn.completed"),
          Stream.runHead,
          Effect.forkScoped,
        );
        yield* Effect.yieldNow;
        yield* instance.adapter.sendTurn({ threadId, input: "many chunks" });
        yield* Fiber.join(terminal);

        const snapshot = yield* instance.adapter.readThread(threadId);
        assert.isAtMost(snapshot.turns[0]?.items.length ?? 0, 200);
        assert.isFalse(containsRawKey(snapshot));
        const storedEvents = (snapshot.turns[0]?.items ?? []).filter(
          (item): item is Record<string, unknown> =>
            typeof item === "object" &&
            item !== null &&
            "type" in item &&
            item.type !== "userMessage",
        );
        assert.isTrue(
          storedEvents.every(
            (event) =>
              event.provider === manifest.driver &&
              event.providerInstanceId === instanceId &&
              event.threadId === threadId &&
              JSON.stringify(event.adapterPackage) ===
                JSON.stringify({
                  id: manifest.id,
                  version: manifest.version,
                  protocolVersion: manifest.protocolVersion,
                }),
          ),
        );
      }),
    ),
  );

  it.effect("rejects incompatible ACP manifest transport before spawning", () =>
    Effect.scoped(
      Effect.gen(function* () {
        const harness = yield* makeHarness();
        const created = yield* createInstance(harness.host, incompatiblePackage).pipe(
          Effect.result,
        );
        assert.equal(created._tag, "Failure");
        assert.equal(yield* Ref.get(harness.spawnCount), 0);
      }),
    ),
  );

  it.effect("rejects unsupported and unavailable operations with typed failures", () =>
    Effect.scoped(
      Effect.gen(function* () {
        const harness = yield* makeHarness();
        const instance = yield* createInstance(harness.host);
        const threadId = ThreadId.make("thread-unsupported");
        yield* instance.adapter.startSession(startInput(threadId));

        const rollback = yield* Effect.exit(instance.adapter.rollbackThread(threadId, 1));
        const structured = yield* Effect.exit(
          instance.adapter.respondToUserInput(threadId, ApprovalRequestId.make("input-1"), {}),
        );
        const attachment = yield* Effect.exit(
          instance.adapter.sendTurn({
            threadId,
            input: "with image",
            attachments: [
              {
                type: "image",
                id: "attachment-1",
                name: "image.png",
                mimeType: "image/png",
                sizeBytes: 1,
              },
            ],
          }),
        );
        const plan = yield* Effect.exit(
          instance.adapter.sendTurn({ threadId, input: "plan", interactionMode: "plan" }),
        );

        assert.isTrue(Exit.isFailure(rollback));
        assert.isTrue(Exit.isFailure(structured));
        assert.isTrue(Exit.isFailure(attachment));
        assert.isTrue(Exit.isFailure(plan));
        assert.include(instance.adapter.capabilities.features, "stream.context");
        assert.notInclude(instance.adapter.capabilities.features, "stream.usage");
        assert.notInclude(instance.adapter.capabilities.features, "input.attachments");
        assert.notInclude(instance.adapter.capabilities.features, "conversation.rollback");
        assert.notInclude(instance.adapter.capabilities.features, "request.structured-input");
      }),
    ),
  );
});
