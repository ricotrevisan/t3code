/**
 * Echo conformance adapter package.
 *
 * Proves the V1 external seam with a real harness process: the package spawns
 * `echoHarness.mjs` through the host process supervisor, frames JSONL
 * requests/responses, and translates harness events into canonical provider
 * runtime events. It receives no server internals — only the V1 host.
 *
 * Protocol reference: see the header of `echoHarness.mjs`.
 */
import * as NodeCrypto from "node:crypto";

import * as Effect from "effect/Effect";
import * as Deferred from "effect/Deferred";
import * as Queue from "effect/Queue";
import * as Schema from "effect/Schema";
import * as Stream from "effect/Stream";

const PROVIDER = "echoHarness";

const failRequest = (operation, result) =>
  Effect.fail({
    _tag: "ProviderAdapterV1Error",
    operation,
    detail: `Echo harness request failed: ${JSON.stringify(result ?? {})}`,
  });

/** ONE JSON object per line protocol helper shared by every session. */
const makeSessionConnection = ({
  process: child,
  provider,
  providerInstanceId,
  threadId,
  events,
}) => {
  const pending = new Map();
  let buffer = "";
  let serverSequence = 0;

  const pushEvent = (harnessEvent) => {
    const event = {
      eventId: NodeCrypto.randomUUID(),
      provider,
      providerInstanceId,
      threadId: harnessEvent.threadId,
      ...(harnessEvent.turnId === undefined ? {} : { turnId: harnessEvent.turnId }),
      createdAt: new Date().toISOString(),
      type: harnessEvent.type,
      payload: harnessEvent.payload ?? {},
    };
    return Queue.offer(events, event).pipe(Effect.asVoid);
  };

  const handleLine = (line) => {
    let message;
    try {
      message = JSON.parse(line);
    } catch {
      return Effect.void;
    }
    if (message.kind === "res") {
      const pendingEntry = pending.get(message.reqId);
      if (pendingEntry) {
        pending.delete(message.reqId);
        return pendingEntry(message);
      }
      return Effect.void;
    }
    if (message.kind === "ev") return pushEvent(message);
    return Effect.void;
  };

  const pump = child.stdout.pipe(
    Stream.decodeText(),
    Stream.map((chunk) => {
      buffer += chunk;
      const lines = [];
      for (;;) {
        const index = buffer.indexOf("\n");
        if (index < 0) break;
        lines.push(buffer.slice(0, index));
        buffer = buffer.slice(index + 1);
      }
      return lines;
    }),
    Stream.flatMap((lines) => Stream.fromIterable(lines)),
    Stream.mapEffect(handleLine),
    Stream.runDrain,
  );

  const request = (body) =>
    Effect.gen(function* () {
      const reqId = `req-${++serverSequence}`;
      const reply = yield* Deferred.make();
      pending.set(reqId, (message) =>
        message.ok
          ? Deferred.succeed(reply, message.result)
          : Deferred.fail(reply, {
              _tag: "ProviderAdapterV1Error",
              operation: "harness-request",
              detail: `Echo harness rejected ${body.type}: ${JSON.stringify(message.result ?? {})}`,
            }),
      );
      const writeResult = yield* child
        .write(new TextEncoder().encode(`${JSON.stringify({ reqId, ...body })}\n`))
        .pipe(Effect.result);
      if (writeResult._tag === "Failure") {
        pending.delete(reqId);
        return yield* Deferred.fail(reply, {
          _tag: "ProviderAdapterV1Error",
          operation: "harness-request",
          detail: `Could not write to echo harness: ${writeResult.failure.detail}`,
        });
      }
      return yield* Deferred.await(reply);
    });

  return {
    threadId,
    request,
    pumpFiber: Effect.forkChild(pump),
    exitCode: child.exitCode,
    expectExit: child.expectExit,
    pushEvent,
  };
};

export default {
  manifest: {
    protocolVersion: 1,
    id: "echo-conformance",
    version: "1.0.0",
    driver: PROVIDER,
    displayName: "Echo Conformance",
    hostProtocol: { minimum: 1, maximum: 1 },
    transport: {
      kind: "supervised-stdio",
      protocol: "jsonl-rpc",
      sessionConcurrency: "one-per-process",
    },
    capabilities: ["turn.interrupt"],
    configSchema: { type: "object", additionalProperties: false },
  },
  configSchema: Schema.Struct({}),
  defaultConfig: () => ({}),
  create: (input, host) =>
    Effect.gen(function* () {
      const harnessUrl = new URL("./echoHarness.mjs", import.meta.url);
      const events = yield* Queue.unbounded();
      /** threadId -> connection */
      const sessions = new Map();
      const now = () => new Date().toISOString();

      const buildSession = (connection, runtimeMode) => ({
        provider: PROVIDER,
        providerInstanceId: input.instanceId,
        status: "ready",
        runtimeMode,
        threadId: connection.threadId,
        createdAt: now(),
        updatedAt: now(),
      });

      const startSession = (sessionInput) =>
        Effect.gen(function* () {
          if (sessions.has(sessionInput.threadId)) {
            return sessions.get(sessionInput.threadId).session;
          }
          const spawnResult = yield* host.processes
            .spawn({
              command: process.execPath,
              args: [harnessUrl.pathname],
              purpose: { kind: "session", threadId: sessionInput.threadId },
            })
            .pipe(Effect.result);
          if (spawnResult._tag === "Failure") {
            return yield* Effect.fail({
              _tag: "ProviderAdapterV1Error",
              operation: "startSession",
              detail: `Could not start echo harness: ${spawnResult.failure.detail}`,
            });
          }
          const connection = makeSessionConnection({
            process: spawnResult.success,
            provider: PROVIDER,
            providerInstanceId: input.instanceId,
            threadId: sessionInput.threadId,
            events,
          });
          const session = buildSession(connection, sessionInput.runtimeMode);
          sessions.set(sessionInput.threadId, { connection, session });
          yield* connection.pumpFiber;
          const result = yield* connection.request({
            type: "start",
            threadId: connection.threadId,
          });
          if (!result?.started) {
            return yield* failRequest("startSession", result);
          }
          return session;
        });

      const requireConnection = (threadId, operation) => {
        const entry = sessions.get(threadId);
        if (!entry) {
          return Effect.fail({
            _tag: "ProviderAdapterV1Error",
            operation,
            detail: `No echo session for thread '${threadId}'.`,
          });
        }
        return Effect.succeed(entry.connection);
      };

      return {
        continuationKey: "echo-conformance",
        snapshot: {
          getSnapshot: Effect.succeed({
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
          }),
          refresh: Effect.succeed({
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
          }),
          streamChanges: Stream.empty,
        },
        adapter: {
          capabilities: { protocolVersion: 1, features: ["turn.interrupt"] },
          startSession,
          sendTurn: (turnInput) =>
            Effect.gen(function* () {
              const connection = yield* requireConnection(turnInput.threadId, "sendTurn");
              const result = yield* connection.request({
                type: "send",
                threadId: turnInput.threadId,
                input: turnInput.input,
                respondAt: "started",
                // Plan-mode turns stay open until an interrupt arrives.
                hold: turnInput.interactionMode === "plan",
              });
              if (!result?.turnId) return yield* failRequest("sendTurn", result);
              const entry = sessions.get(turnInput.threadId);
              entry.session = { ...entry.session, activeTurnId: result.turnId, updatedAt: now() };
              return {
                threadId: turnInput.threadId,
                turnId: result.turnId,
                resumeCursor: { echo: connection.threadId },
              };
            }),
          interruptTurn: (threadId) =>
            Effect.gen(function* () {
              const connection = yield* requireConnection(threadId, "interruptTurn");
              yield* connection.request({ type: "interrupt", threadId });
              const entry = sessions.get(threadId);
              entry.session = { ...entry.session, activeTurnId: undefined, updatedAt: now() };
            }),
          respondToRequest: () =>
            Effect.fail({
              _tag: "ProviderAdapterV1Error",
              operation: "respondToRequest",
              detail: "Echo harness does not support approvals.",
            }),
          respondToUserInput: () =>
            Effect.fail({
              _tag: "ProviderAdapterV1Error",
              operation: "respondToUserInput",
              detail: "Echo harness does not support structured input.",
            }),
          stopSession: (threadId) =>
            Effect.gen(function* () {
              const entry = sessions.get(threadId);
              if (!entry) return;
              sessions.delete(threadId);
              yield* entry.connection.expectExit;
              yield* entry.connection.request({ type: "stop", threadId });
              yield* entry.connection.exitCode;
            }),
          listSessions: () =>
            Effect.sync(() => Array.from(sessions.values(), (entry) => entry.session)),
          hasSession: (threadId) => Effect.succeed(sessions.has(threadId)),
          readThread: (threadId) =>
            Effect.gen(function* () {
              const connection = yield* requireConnection(threadId, "readThread");
              const result = yield* connection.request({ type: "readThread", threadId });
              return { threadId, turns: result?.turns ?? [] };
            }),
          rollbackThread: (threadId, numTurns) =>
            Effect.gen(function* () {
              const connection = yield* requireConnection(threadId, "rollbackThread");
              const result = yield* connection.request({ type: "rollback", threadId, numTurns });
              return { threadId, turns: result?.turns ?? [] };
            }),
          stopAll: () =>
            Effect.forEach(Array.from(sessions.keys()), (threadId) =>
              Effect.gen(function* () {
                const entry = sessions.get(threadId);
                sessions.delete(threadId);
                yield* entry.connection.expectExit;
                yield* entry.connection.request({ type: "stop", threadId });
                yield* entry.connection.exitCode;
              }),
            ).pipe(Effect.asVoid),
          streamEvents: Stream.fromQueue(events),
        },
      };
    }),
};
