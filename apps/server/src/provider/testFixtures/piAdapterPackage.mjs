/**
 * Pi RPC adapter package.
 *
 * Translates the Pi coding agent's RPC mode (JSONL commands/events over a
 * supervised stdio process) into T3's canonical provider sessions, commands,
 * and runtime events. Talks only to the V1 host: process supervisor plus
 * this package's own state. No T3 internals.
 *
 * Real deployment: `binaryPath: "pi"`, `args: ["--mode", "rpc"]`. The
 * conformance fixture defaults `binaryPath` to Node and points `args` at the
 * fake harness (`piHarness.mjs`).
 *
 * Advertised gaps (advertised, not emulated): no approvals (Pi permissions
 * live behind a controlled extension), no rollback (Pi forks sessions
 * instead), attachments deferred. Session rollback uses Pi's entry-id
 * cursor only as a read cursor.
 */
import * as NodeCrypto from "node:crypto";
import * as NodeURL from "node:url";

import * as Deferred from "effect/Deferred";
import * as Effect from "effect/Effect";
import * as Queue from "effect/Queue";
import * as Schema from "effect/Schema";
import * as Stream from "effect/Stream";

const PROVIDER = "piRpc";
const CAPABILITIES = [
  "session.resume",
  "turn.steer",
  "turn.interrupt",
  "request.structured-input",
  "model.discovery",
  "model.switch",
  "reasoning.selection",
  "stream.reasoning",
  "stream.tool-lifecycle",
  "stream.usage",
];

const adapterError = (operation, detail) =>
  Effect.fail({ _tag: "ProviderAdapterV1Error", operation, detail });

const itemTypeForTool = (toolName) =>
  toolName === "bash" ? "command_execution" : "dynamic_tool_call";

/** One supervised Pi process: framed request/response plus event translation. */
const makeConnection = ({ process: child, providerInstanceId, threadId, events }) => {
  const pending = new Map();
  let buffer = "";
  let requestSequence = 0;
  /** Client-assigned canonical turn id while a prompt is in flight. */
  let activeTurnId = undefined;
  let abortRequested = false;
  let turnStartedEmitted = false;
  let lastAssistantMessage;
  let transportClosed = false;
  const uiRequests = new Map();

  const base = () => ({
    eventId: NodeCrypto.randomUUID(),
    provider: PROVIDER,
    providerInstanceId,
    threadId,
    createdAt: new Date().toISOString(),
  });

  const pushEvent = (event) => Queue.offer(events, { ...base(), ...event }).pipe(Effect.asVoid);

  const withTurn = (event) => ({
    ...event,
    ...(activeTurnId === undefined ? {} : { turnId: activeTurnId }),
  });

  const emitTurnStarted = () => {
    if (turnStartedEmitted || activeTurnId === undefined) return Effect.void;
    turnStartedEmitted = true;
    return pushEvent(withTurn({ type: "turn.started", payload: {} }));
  };

  const resetTurn = () => {
    activeTurnId = undefined;
    turnStartedEmitted = false;
    abortRequested = false;
    lastAssistantMessage = undefined;
  };

  const settleTurn = () =>
    Effect.gen(function* () {
      if (activeTurnId === undefined) return;
      yield* emitTurnStarted();
      const message = lastAssistantMessage;
      const terminal = withTurn(
        abortRequested || message?.stopReason === "aborted"
          ? { type: "turn.aborted", payload: { reason: "aborted by user" } }
          : {
              type: "turn.completed",
              payload: {
                state: message?.stopReason === "error" ? "failed" : "completed",
                ...(message?.stopReason === undefined ? {} : { stopReason: message.stopReason }),
                ...(message?.errorMessage ? { errorMessage: message.errorMessage } : {}),
              },
            },
      );
      resetTurn();
      yield* pushEvent(terminal);
    });

  const emitUsage = (usage) =>
    usage?.totalTokens === undefined
      ? Effect.void
      : pushEvent(
          withTurn({
            type: "thread.token-usage.updated",
            payload: {
              usage: {
                usedTokens: usage.totalTokens,
                inputTokens: usage.input,
                outputTokens: usage.output,
              },
            },
          }),
        );

  const handlePiEvent = (event) => {
    switch (event.type) {
      case "agent_start":
      case "turn_start":
        return emitTurnStarted();
      case "message_update": {
        const delta = event.assistantMessageEvent ?? {};
        if (delta.type === "text_delta") {
          return Effect.all([
            emitTurnStarted(),
            pushEvent(
              withTurn({
                type: "content.delta",
                payload: {
                  streamKind: "assistant_text",
                  delta: delta.delta,
                  contentIndex: delta.contentIndex,
                },
              }),
            ),
          ]).pipe(Effect.asVoid);
        }
        if (delta.type === "thinking_delta") {
          return Effect.all([
            emitTurnStarted(),
            pushEvent(
              withTurn({
                type: "content.delta",
                payload: {
                  streamKind: "reasoning_text",
                  delta: delta.delta,
                  contentIndex: delta.contentIndex,
                },
              }),
            ),
          ]).pipe(Effect.asVoid);
        }
        if (delta.type === "toolcall_start") {
          return Effect.all([
            emitTurnStarted(),
            pushEvent(
              withTurn({
                type: "item.started",
                itemId: delta.id,
                payload: { itemType: itemTypeForTool(delta.toolName), title: delta.toolName },
              }),
            ),
          ]).pipe(Effect.asVoid);
        }
        return Effect.void;
      }
      case "tool_execution_end":
        return pushEvent(
          withTurn({
            type: "item.completed",
            itemId: event.toolCallId,
            payload: {
              itemType: itemTypeForTool(event.toolName),
              status: event.isError === true ? "failed" : "completed",
              data: event.result,
            },
          }),
        );
      case "message_end":
        if (event.message?.role !== "assistant") return Effect.void;
        lastAssistantMessage = event.message;
        return emitUsage(event.message.usage);
      case "turn_end":
        lastAssistantMessage = event.message;
        return Effect.void;
      case "agent_settled":
        return settleTurn();
      case "extension_ui_request": {
        if (!["select", "confirm", "input", "editor"].includes(event.method)) {
          // RPC also uses this envelope for fire-and-forget display updates.
          return Effect.void;
        }
        uiRequests.set(event.id, event.method);
        const options = event.method === "confirm" ? ["Yes", "No"] : (event.options ?? []);
        return pushEvent(
          withTurn({
            type: "user-input.requested",
            requestId: event.id,
            payload: {
              questions: [
                {
                  id: event.id,
                  header: event.title || event.method,
                  question: event.message || event.title || event.method,
                  options: options.map((option) => ({
                    label: String(option),
                    description: String(option),
                  })),
                  multiSelect: false,
                },
              ],
            },
          }),
        );
      }
      default:
        return Effect.void;
    }
  };

  const pump = child.stdout.pipe(
    Stream.decodeText(),
    Stream.map((chunk) => {
      buffer += chunk;
      const lines = [];
      for (;;) {
        const index = buffer.indexOf("\n");
        if (index < 0) break;
        lines.push(buffer.slice(0, index).replace(/\r$/, ""));
        buffer = buffer.slice(index + 1);
      }
      return lines;
    }),
    Stream.flatMap((lines) => Stream.fromIterable(lines)),
    Stream.mapEffect((line) =>
      Effect.gen(function* () {
        let message;
        try {
          message = JSON.parse(line);
        } catch {
          return;
        }
        if (message.type === "response") {
          const entry = message.id === undefined ? undefined : pending.get(message.id);
          if (entry) {
            pending.delete(message.id);
            if (message.success === false) {
              yield* Deferred.fail(entry, {
                _tag: "ProviderAdapterV1Error",
                operation: String(message.command ?? "pi-command"),
                detail: String(message.error ?? "Pi command failed"),
              });
            } else {
              yield* Deferred.succeed(entry, message.data ?? {});
            }
          }
          return;
        }
        yield* handlePiEvent(message);
      }),
    ),
    Stream.runDrain,
    Effect.ensuring(
      Effect.gen(function* () {
        transportClosed = true;
        for (const reply of pending.values()) {
          yield* Deferred.fail(reply, {
            _tag: "ProviderAdapterV1Error",
            operation: "pi-rpc",
            detail: "Pi RPC output closed before the command completed.",
          });
        }
        pending.clear();
      }),
    ),
  );

  const request = (command) =>
    Effect.gen(function* () {
      if (transportClosed)
        return yield* adapterError(String(command.type), "Pi RPC output is closed.");
      requestSequence += 1;
      const id = `t3-${requestSequence}`;
      const reply = yield* Deferred.make();
      pending.set(id, reply);
      const writeResult = yield* child
        .write(new TextEncoder().encode(`${JSON.stringify({ id, ...command })}\n`))
        .pipe(Effect.result);
      if (writeResult._tag === "Failure") {
        pending.delete(id);
        return yield* adapterError(
          String(command.type),
          `Could not write to Pi process: ${writeResult.failure.detail}`,
        );
      }
      return yield* Deferred.await(reply);
    });

  const notify = (command) =>
    Effect.gen(function* () {
      const writeResult = yield* child
        .write(new TextEncoder().encode(`${JSON.stringify(command)}\n`))
        .pipe(Effect.result);
      if (writeResult._tag === "Failure") {
        return yield* adapterError(
          String(command.type),
          `Could not write to Pi process: ${writeResult.failure.detail}`,
        );
      }
    });

  return {
    threadId,
    request,
    notify,
    pumpFiber: Effect.forkChild(pump),
    exitCode: child.exitCode,
    expectExit: child.expectExit,
    close: child.close,
    /** Reserve an id before the request so early events retain their owner. */
    beginTurn: (turnId) => {
      resetTurn();
      activeTurnId = turnId;
    },
    resetTurn,
    settleHandledPrompt: (turnId, state) =>
      activeTurnId === turnId && !turnStartedEmitted && !state.isStreaming && !state.isCompacting
        ? settleTurn()
        : Effect.void,
    uiRequests,
    markAbortRequested: () => {
      abortRequested = true;
    },
    get activeTurnId() {
      return activeTurnId;
    },
  };
};

const toServerModels = (models) =>
  models.map((model) => ({
    slug: `${model.provider}/${model.id}`,
    name: String(model.name ?? model.id),
    isCustom: false,
    capabilities: {},
  }));

export default {
  manifest: {
    protocolVersion: 1,
    id: "pi-rpc",
    version: "1.0.0",
    driver: PROVIDER,
    displayName: "Pi RPC",
    hostProtocol: { minimum: 1, maximum: 1 },
    transport: {
      kind: "supervised-stdio",
      protocol: "jsonl-rpc",
      sessionConcurrency: "one-per-process",
    },
    capabilities: CAPABILITIES,
    configSchema: {
      type: "object",
      properties: {
        binaryPath: { type: "string" },
        args: { type: "array", items: { type: "string" } },
        model: { type: "string" },
        thinkingLevel: { type: "string" },
      },
    },
  },
  configSchema: Schema.Struct({
    binaryPath: Schema.optional(Schema.String),
    args: Schema.optional(Schema.Array(Schema.String)),
    model: Schema.optional(Schema.String),
    thinkingLevel: Schema.optional(Schema.String),
  }),
  defaultConfig: () => ({}),
  create: (input, host) =>
    Effect.gen(function* () {
      const events = yield* Queue.unbounded();
      const sessions = new Map();
      const now = () => new Date().toISOString();

      const spawnConnection = (threadId, cwd) =>
        Effect.gen(function* () {
          const harnessUrl = new URL("./piHarness.mjs", import.meta.url);
          const binaryPath = input.config.binaryPath ?? process.execPath;
          const args =
            input.config.args ??
            (input.config.binaryPath === undefined ? [NodeURL.fileURLToPath(harnessUrl)] : []);
          const spawnResult = yield* host.processes
            .spawn({
              command: binaryPath,
              args,
              ...(cwd === undefined ? {} : { cwd }),
              environment: input.environment,
              purpose: { kind: "session", threadId },
            })
            .pipe(Effect.result);
          if (spawnResult._tag === "Failure") {
            return yield* adapterError(
              "startSession",
              `Could not start Pi process: ${spawnResult.failure.detail}`,
            );
          }
          const connection = makeConnection({
            process: spawnResult.success,
            providerInstanceId: input.instanceId,
            threadId,
            events,
          });
          yield* connection.pumpFiber;
          return connection;
        });

      const applyModelSelection = (connection, modelSelection) =>
        Effect.gen(function* () {
          if (modelSelection === undefined) return;
          const separator = modelSelection.model.indexOf("/");
          if (separator < 1 || separator === modelSelection.model.length - 1) {
            return yield* adapterError(
              "set_model",
              "Pi models must use a provider/modelId selection.",
            );
          }
          yield* connection.request({
            type: "set_model",
            provider: modelSelection.model.slice(0, separator),
            modelId: modelSelection.model.slice(separator + 1),
          });
          for (const option of modelSelection.options ?? []) {
            if (option.id === "reasoningEffort") {
              yield* connection.request({
                type: "set_thinking_level",
                level: String(option.value),
              });
            }
          }
        });

      const buildSession = (connection, threadId, runtimeMode, resumeCursor) => ({
        provider: PROVIDER,
        providerInstanceId: input.instanceId,
        status: "ready",
        runtimeMode,
        threadId,
        ...(resumeCursor === undefined ? {} : { resumeCursor }),
        createdAt: now(),
        updatedAt: now(),
      });

      const startSession = (sessionInput) =>
        Effect.gen(function* () {
          const existing = sessions.get(sessionInput.threadId);
          if (existing) return existing.session;
          const connection = yield* spawnConnection(sessionInput.threadId, sessionInput.cwd);
          return yield* Effect.gen(function* () {
            const resumeCursor =
              sessionInput.resumeCursor?.sessionFile === undefined
                ? undefined
                : { sessionFile: sessionInput.resumeCursor.sessionFile };
            if (resumeCursor !== undefined) {
              const switched = yield* connection.request({
                type: "switch_session",
                sessionPath: resumeCursor.sessionFile,
              });
              if (switched.cancelled) {
                return yield* adapterError("startSession", "Pi session resume was cancelled.");
              }
            }
            yield* applyModelSelection(
              connection,
              sessionInput.modelSelection ??
                (input.config.model === undefined ? undefined : { model: input.config.model }),
            );
            if (
              input.config.thinkingLevel !== undefined &&
              sessionInput.modelSelection === undefined
            ) {
              yield* connection.request({
                type: "set_thinking_level",
                level: input.config.thinkingLevel,
              });
            }
            const state = yield* connection.request({ type: "get_state" });
            const session = buildSession(
              connection,
              sessionInput.threadId,
              sessionInput.runtimeMode,
              {
                sessionFile: state.sessionFile,
              },
            );
            sessions.set(sessionInput.threadId, { connection, session });
            return session;
          }).pipe(Effect.onError(() => connection.close.pipe(Effect.ignore)));
        });

      const requireConnection = (threadId, operation) => {
        const entry = sessions.get(threadId);
        if (!entry) {
          return adapterError(operation, `No Pi session for thread '${threadId}'.`);
        }
        return Effect.succeed(entry.connection);
      };

      const runPrompt = (connection, threadId, command) =>
        Effect.gen(function* () {
          const turnId = `pi-turn-${NodeCrypto.randomUUID()}`;
          connection.beginTurn(turnId);
          const accepted = yield* connection.request(command).pipe(Effect.result);
          if (accepted._tag === "Failure") {
            connection.resetTurn();
            return yield* Effect.fail(accepted.failure);
          }
          // Pi starts normal runs synchronously after preflight acceptance. An
          // accepted extension/input handler may instead finish without a run.
          const state = yield* connection.request({ type: "get_state" });
          yield* connection.settleHandledPrompt(turnId, state);
          return {
            threadId,
            turnId,
            resumeCursor: { sessionFile: sessions.get(threadId)?.session.resumeCursor.sessionFile },
          };
        });

      return {
        continuationKey: "pi-rpc",
        snapshot: {
          getSnapshot: Effect.gen(function* () {
            const probe = yield* host.processes
              .spawn({
                command: input.config.binaryPath ?? process.execPath,
                args:
                  input.config.args ??
                  (input.config.binaryPath === undefined
                    ? [NodeURL.fileURLToPath(new URL("./piHarness.mjs", import.meta.url))]
                    : []),
                environment: input.environment,
                purpose: { kind: "probe" },
              })
              .pipe(Effect.result);
            let models = [];
            if (probe._tag === "Success") {
              const probeConnection = makeConnection({
                process: probe.success,
                providerInstanceId: input.instanceId,
                threadId: "probe",
                events,
              });
              yield* probeConnection.pumpFiber;
              const discovered = yield* probeConnection.request({ type: "get_available_models" });
              models = toServerModels(discovered.models ?? []);
              yield* probe.success.close;
            }
            return {
              instanceId: input.instanceId,
              driver: PROVIDER,
              displayName: input.displayName,
              enabled: input.enabled,
              installed: true,
              version: "1.0.0",
              status: input.enabled ? "ready" : "disabled",
              auth: { status: "unknown" },
              checkedAt: now(),
              availability: "available",
              models,
              slashCommands: [],
              skills: [],
            };
          }),
          refresh: Effect.sync(() => ({
            instanceId: input.instanceId,
            driver: PROVIDER,
            displayName: input.displayName,
            enabled: input.enabled,
            installed: true,
            version: "1.0.0",
            status: input.enabled ? "ready" : "disabled",
            auth: { status: "unknown" },
            checkedAt: now(),
            availability: "available",
            models: [],
            slashCommands: [],
            skills: [],
          })),
          streamChanges: Stream.empty,
        },
        adapter: {
          capabilities: { protocolVersion: 1, features: CAPABILITIES },
          startSession,
          sendTurn: (turnInput) =>
            Effect.gen(function* () {
              const connection = yield* requireConnection(turnInput.threadId, "sendTurn");
              const activeTurnId = connection.activeTurnId;
              if (activeTurnId !== undefined) {
                // Mid-turn message: Pi steers the running agent instead of
                // opening a new canonical turn. Capture the id before the
                // request because Pi may settle and clear it before replying.
                yield* connection.request({ type: "steer", message: turnInput.input ?? "" });
                return {
                  threadId: turnInput.threadId,
                  turnId: activeTurnId,
                  resumeCursor: {
                    sessionFile: sessions.get(turnInput.threadId).session.resumeCursor.sessionFile,
                  },
                };
              }
              if (turnInput.modelSelection !== undefined) {
                yield* applyModelSelection(connection, turnInput.modelSelection);
              }
              return yield* runPrompt(connection, turnInput.threadId, {
                type: "prompt",
                message: turnInput.input ?? "",
              });
            }),
          interruptTurn: (threadId) =>
            Effect.gen(function* () {
              const connection = yield* requireConnection(threadId, "interruptTurn");
              connection.markAbortRequested();
              yield* connection.request({ type: "abort" });
            }),
          respondToRequest: () =>
            adapterError(
              "respondToRequest",
              "Pi approvals require a controlled extension and are not negotiated.",
            ),
          respondToUserInput: (threadId, requestId, answers) =>
            Effect.gen(function* () {
              const connection = yield* requireConnection(threadId, "respondToUserInput");
              const method = connection.uiRequests.get(requestId);
              if (method === undefined) {
                return yield* adapterError("respondToUserInput", "Unknown Pi UI request.");
              }
              const answer = Object.values(answers)[0];
              const response =
                answer === undefined
                  ? { type: "extension_ui_response", id: requestId, cancelled: true }
                  : method === "confirm"
                    ? {
                        type: "extension_ui_response",
                        id: requestId,
                        confirmed: answer === true || answer === "Yes" || answer === "true",
                      }
                    : { type: "extension_ui_response", id: requestId, value: String(answer) };
              yield* connection.notify(response);
              connection.uiRequests.delete(requestId);
              yield* Queue.offer(events, {
                eventId: NodeCrypto.randomUUID(),
                provider: PROVIDER,
                providerInstanceId: input.instanceId,
                threadId,
                createdAt: now(),
                type: "user-input.resolved",
                requestId,
                payload: { answers: { value: answer ?? null } },
              }).pipe(Effect.asVoid);
            }),
          stopSession: (threadId) =>
            Effect.gen(function* () {
              const entry = sessions.get(threadId);
              if (!entry) return;
              sessions.delete(threadId);
              yield* entry.connection.expectExit;
              yield* entry.connection.close;
              yield* entry.connection.exitCode;
            }),
          listSessions: () =>
            Effect.sync(() => Array.from(sessions.values(), (entry) => entry.session)),
          hasSession: (threadId) => Effect.succeed(sessions.has(threadId)),
          readThread: (threadId) =>
            Effect.gen(function* () {
              const connection = yield* requireConnection(threadId, "readThread");
              const result = yield* connection.request({ type: "get_entries" });
              const byId = new Map((result.entries ?? []).map((entry) => [entry.id, entry]));
              const branch = [];
              const visited = new Set();
              let cursor = result.leafId;
              while (cursor != null && !visited.has(cursor)) {
                visited.add(cursor);
                const entry = byId.get(cursor);
                if (entry === undefined) break;
                branch.push(entry);
                cursor = entry.parentId;
              }
              return {
                threadId,
                turns: branch
                  .toReversed()
                  .filter((entry) => entry.type === "message")
                  .map((entry) => ({
                    id: entry.id,
                    items: [entry.message],
                  })),
              };
            }),
          rollbackThread: () =>
            adapterError(
              "rollbackThread",
              "Pi rollback forks sessions and is not negotiated for protocol 1.",
            ),
          stopAll: () =>
            Effect.forEach(Array.from(sessions.keys()), (threadId) =>
              Effect.gen(function* () {
                const entry = sessions.get(threadId);
                sessions.delete(threadId);
                yield* entry.connection.expectExit;
                yield* entry.connection.close;
                yield* entry.connection.exitCode;
              }),
            ).pipe(Effect.asVoid),
          streamEvents: Stream.fromQueue(events),
        },
      };
    }),
};
