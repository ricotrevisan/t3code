// @effect-diagnostics nodeBuiltinImport:off
import * as NodeCrypto from "node:crypto";
import {
  EventId,
  ProviderDriverKind,
  RuntimeItemId,
  RuntimeRequestId,
  ThreadId,
  TurnId,
  type ModelSelection,
  type ProviderAdapterProtocolCapabilitiesV1,
  type ProviderRuntimeEvent,
  type ProviderSession,
  type ProviderUserInputAnswers,
  type ServerProvider,
} from "@t3tools/contracts";
import {
  ProviderAdapterV1Error,
  type ProviderAdapterCreateInputV1,
  type ProviderAdapterHostV2,
  type ProviderAdapterProcessV1,
  type ProviderAdapterThreadSnapshotV1,
  type ProviderAdapterV1,
} from "@t3tools/provider-adapter";
import * as DateTime from "effect/DateTime";
import * as Deferred from "effect/Deferred";
import * as Effect from "effect/Effect";
import * as Exit from "effect/Exit";
import * as PubSub from "effect/PubSub";
import * as Result from "effect/Result";
import * as Schema from "effect/Schema";
import * as Semaphore from "effect/Semaphore";
import * as Scope from "effect/Scope";
import * as Stream from "effect/Stream";

import type { PiProviderAdapterConfig } from "./index.ts";

const PROVIDER = ProviderDriverKind.make("piRpc");
const PROBE_THREAD_ID = ThreadId.make("pi-provider-probe");

export const PI_PROVIDER_ADAPTER_CAPABILITIES = {
  protocolVersion: 1,
  features: [
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
  ],
} as const satisfies ProviderAdapterProtocolCapabilitiesV1;

const PiState = Schema.Struct({
  sessionFile: Schema.String,
  sessionId: Schema.String,
  isStreaming: Schema.Boolean,
  isCompacting: Schema.Boolean,
  thinkingLevel: Schema.optional(Schema.String),
});
const PiModels = Schema.Struct({
  models: Schema.Array(
    Schema.Struct({
      provider: Schema.String,
      id: Schema.String,
      name: Schema.optional(Schema.String),
    }),
  ),
});
const PiThinkingLevels = Schema.Struct({ levels: Schema.Array(Schema.String) });
const PiSwitchSessionResult = Schema.Struct({ cancelled: Schema.optional(Schema.Boolean) });
const PiEntries = Schema.Struct({
  entries: Schema.Array(
    Schema.Struct({
      id: Schema.String,
      parentId: Schema.optional(Schema.NullOr(Schema.String)),
      type: Schema.String,
      message: Schema.optional(Schema.Unknown),
    }),
  ),
  leafId: Schema.optional(Schema.NullOr(Schema.String)),
});

const decodeState = Schema.decodeUnknownEffect(PiState);
const decodeModels = Schema.decodeUnknownEffect(PiModels);
const decodeThinkingLevels = Schema.decodeUnknownEffect(PiThinkingLevels);
const decodeSwitchSession = Schema.decodeUnknownEffect(PiSwitchSessionResult);
const decodeEntries = Schema.decodeUnknownEffect(PiEntries);
const decodeJsonExit = Schema.decodeUnknownExit(Schema.fromJsonString(Schema.Unknown));
const encodeJson = Schema.encodeUnknownEffect(Schema.fromJsonString(Schema.Unknown));
const isAdapterError = Schema.is(ProviderAdapterV1Error);
const nowIso = Effect.map(DateTime.now, DateTime.formatIso);

interface PiResumeCursor {
  readonly sessionFile: string;
  readonly sessionId: string;
}

interface PiAssistantMessage {
  readonly stopReason?: string | null | undefined;
  readonly errorMessage?: string | undefined;
  readonly usage?: {
    readonly input?: number | undefined;
    readonly output?: number | undefined;
    readonly totalTokens?: number | undefined;
  };
}

interface PiConnection {
  readonly threadId: ThreadId;
  readonly request: (
    command: Readonly<Record<string, unknown>>,
  ) => Effect.Effect<unknown, ProviderAdapterV1Error>;
  readonly notify: (
    command: Readonly<Record<string, unknown>>,
  ) => Effect.Effect<void, ProviderAdapterV1Error>;
  readonly startPump: Effect.Effect<void>;
  readonly exitCode: ProviderAdapterProcessV1["exitCode"];
  readonly expectExit: ProviderAdapterProcessV1["expectExit"];
  readonly close: ProviderAdapterProcessV1["close"];
  readonly scope: Scope.Closeable;
  readonly beginTurn: (turnId: TurnId) => void;
  readonly resetTurn: () => void;
  readonly settleHandledPrompt: (turnId: TurnId, state: typeof PiState.Type) => Effect.Effect<void>;
  readonly uiRequests: Map<string, "select" | "confirm" | "input" | "editor">;
  readonly markAbortRequested: () => void;
  readonly clearAbortRequested: () => void;
  readonly activeTurnId: TurnId | undefined;
  readonly isClosed: boolean;
}

interface PiSessionEntry {
  readonly connection: PiConnection;
  readonly session: ProviderSession;
  readonly resumeCursor: PiResumeCursor;
}

type PiEventInput<Event extends ProviderRuntimeEvent = ProviderRuntimeEvent> = Event extends unknown
  ? Omit<
      Event,
      "eventId" | "provider" | "providerInstanceId" | "adapterPackage" | "threadId" | "createdAt"
    >
  : never;

function adapterError(operation: string, detail: string, cause?: unknown) {
  return new ProviderAdapterV1Error({
    operation,
    detail,
    ...(cause === undefined ? {} : { cause }),
  });
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function stringField(record: Record<string, unknown>, key: string): string | undefined {
  const value = record[key];
  return typeof value === "string" ? value : undefined;
}

function numberField(record: Record<string, unknown>, key: string): number | undefined {
  const value = record[key];
  return typeof value === "number" ? value : undefined;
}

const MAX_TOOL_EVENT_DATA_CHARS = 64_000;

function boundedToolData(value: unknown): unknown {
  try {
    const encoded = JSON.stringify(value);
    if (encoded.length <= MAX_TOOL_EVENT_DATA_CHARS) return value;
    return {
      truncated: true,
      preview: encoded.slice(0, MAX_TOOL_EVENT_DATA_CHARS),
    };
  } catch {
    return { truncated: true, preview: "Tool data could not be serialized." };
  }
}

function parseResumeCursor(raw: unknown): PiResumeCursor | undefined {
  if (!isRecord(raw)) return undefined;
  const sessionFile = stringField(raw, "sessionFile")?.trim();
  const sessionId = stringField(raw, "sessionId")?.trim();
  return sessionFile && sessionId ? { sessionFile, sessionId } : undefined;
}

function parseAssistantMessage(raw: unknown): PiAssistantMessage | undefined {
  if (!isRecord(raw) || raw.role !== "assistant") return undefined;
  const usage = isRecord(raw.usage)
    ? {
        input: numberField(raw.usage, "input"),
        output: numberField(raw.usage, "output"),
        totalTokens: numberField(raw.usage, "totalTokens"),
      }
    : undefined;
  return {
    ...(typeof raw.stopReason === "string" || raw.stopReason === null
      ? { stopReason: raw.stopReason }
      : {}),
    ...(typeof raw.errorMessage === "string" ? { errorMessage: raw.errorMessage } : {}),
    ...(usage === undefined ? {} : { usage }),
  };
}

function itemTypeForTool(toolName: string): "command_execution" | "dynamic_tool_call" {
  return toolName === "bash" ? "command_execution" : "dynamic_tool_call";
}

const FORBIDDEN_PI_LAUNCH_FLAGS = [
  "--",
  "--help",
  "-h",
  "--version",
  "-v",
  "--export",
  "--list-models",
  "--mode",
  "--session-dir",
  "--session",
  "--session-id",
  "--fork",
  "--continue",
  "--resume",
  "--no-session",
  "-c",
  "-r",
] as const;

function forbiddenLaunchFlag(args: ReadonlyArray<string>): string | undefined {
  return args.find((arg) =>
    FORBIDDEN_PI_LAUNCH_FLAGS.some((flag) => arg === flag || arg.startsWith(`${flag}=`)),
  );
}

const launchArgs = (args: ReadonlyArray<string>, sessionDirectory: string) => [
  ...args,
  "--mode",
  "rpc",
  "--session-dir",
  sessionDirectory,
];

function commandAppearsMissing(cause: unknown, seen = new Set<unknown>()): boolean {
  if (seen.has(cause)) return false;
  seen.add(cause);
  if (typeof cause !== "object" || cause === null) {
    return /(?:ENOENT|not found|could not find|cannot find)/i.test(String(cause));
  }
  const record = cause as Record<string, unknown>;
  if (record.code === "ENOENT" || record.reason === "NotFound") return true;
  const detail = [record.message, record.detail]
    .filter((value): value is string => typeof value === "string")
    .join(" ");
  return (
    /(?:ENOENT|not found|could not find|cannot find)/i.test(detail) ||
    commandAppearsMissing(record.cause, seen)
  );
}

function makeConnection(input: {
  readonly process: ProviderAdapterProcessV1;
  readonly providerInstanceId: ProviderAdapterCreateInputV1<PiProviderAdapterConfig>["instanceId"];
  readonly threadId: ThreadId;
  readonly events: PubSub.PubSub<ProviderRuntimeEvent>;
  readonly scope: Scope.Closeable;
}): PiConnection {
  const child = input.process;
  const pending = new Map<string, Deferred.Deferred<unknown, ProviderAdapterV1Error>>();
  const uiRequests = new Map<string, "select" | "confirm" | "input" | "editor">();
  let buffer = "";
  let requestSequence = 0;
  let activeTurnId: TurnId | undefined;
  let abortRequested = false;
  let turnStartedEmitted = false;
  let lastAssistantMessage: PiAssistantMessage | undefined;
  let transportClosed = false;

  const pushEvent = (event: PiEventInput) =>
    Effect.gen(function* () {
      const stamped = {
        eventId: EventId.make(yield* Effect.sync(NodeCrypto.randomUUID)),
        provider: PROVIDER,
        providerInstanceId: input.providerInstanceId,
        threadId: input.threadId,
        createdAt: yield* nowIso,
        ...event,
      } as ProviderRuntimeEvent;
      yield* PubSub.publish(input.events, stamped);
    });

  const withTurn = (event: PiEventInput): PiEventInput => ({
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
      const terminal: PiEventInput =
        abortRequested || message?.stopReason === "aborted"
          ? withTurn({ type: "turn.aborted", payload: { reason: "aborted by user" } })
          : withTurn({
              type: "turn.completed",
              payload: {
                state: message?.stopReason === "error" ? "failed" : "completed",
                ...(message?.stopReason == null ? {} : { stopReason: message.stopReason }),
                ...(message?.errorMessage ? { errorMessage: message.errorMessage } : {}),
              },
            });
      for (const requestId of uiRequests.keys()) {
        yield* pushEvent(
          withTurn({
            type: "user-input.resolved",
            requestId: RuntimeRequestId.make(requestId),
            payload: { answers: {} },
          }),
        );
      }
      uiRequests.clear();
      resetTurn();
      yield* pushEvent(terminal);
    });

  const emitUsage = (usage: PiAssistantMessage["usage"]) =>
    usage?.totalTokens === undefined
      ? Effect.void
      : pushEvent(
          withTurn({
            type: "thread.token-usage.updated",
            payload: {
              usage: {
                usedTokens: usage.totalTokens,
                ...(usage.input === undefined ? {} : { inputTokens: usage.input }),
                ...(usage.output === undefined ? {} : { outputTokens: usage.output }),
              },
            },
          }),
        );

  const handlePiEvent = (raw: unknown): Effect.Effect<void> => {
    if (!isRecord(raw)) return Effect.void;
    const type = stringField(raw, "type");
    switch (type) {
      case "agent_start":
      case "turn_start":
        return emitTurnStarted();
      case "message_update": {
        if (!isRecord(raw.assistantMessageEvent)) return Effect.void;
        const delta = raw.assistantMessageEvent;
        const deltaType = stringField(delta, "type");
        if (deltaType === "text_delta" || deltaType === "thinking_delta") {
          const text = stringField(delta, "delta");
          if (text === undefined) return Effect.void;
          const contentIndex = numberField(delta, "contentIndex");
          return emitTurnStarted().pipe(
            Effect.andThen(
              pushEvent(
                withTurn({
                  type: "content.delta",
                  payload: {
                    streamKind: deltaType === "text_delta" ? "assistant_text" : "reasoning_text",
                    delta: text,
                    ...(contentIndex === undefined ? {} : { contentIndex }),
                  },
                }),
              ),
            ),
          );
        }
        if (deltaType === "toolcall_start") {
          const id = stringField(delta, "id");
          const toolName = stringField(delta, "toolName");
          if (id === undefined || toolName === undefined) return Effect.void;
          return emitTurnStarted().pipe(
            Effect.andThen(
              pushEvent(
                withTurn({
                  type: "item.started",
                  itemId: RuntimeItemId.make(id),
                  payload: { itemType: itemTypeForTool(toolName), title: toolName },
                }),
              ),
            ),
          );
        }
        return Effect.void;
      }
      case "tool_execution_start":
      case "tool_execution_update": {
        const toolCallId = stringField(raw, "toolCallId");
        const toolName = stringField(raw, "toolName");
        if (toolCallId === undefined || toolName === undefined) return Effect.void;
        return emitTurnStarted().pipe(
          Effect.andThen(
            pushEvent(
              withTurn({
                type: "item.updated",
                itemId: RuntimeItemId.make(toolCallId),
                payload: {
                  itemType: itemTypeForTool(toolName),
                  status: "inProgress",
                  title: toolName,
                  data: boundedToolData(
                    type === "tool_execution_start"
                      ? { args: raw.args }
                      : { args: raw.args, result: raw.partialResult },
                  ),
                },
              }),
            ),
          ),
        );
      }
      case "tool_execution_end": {
        const toolCallId = stringField(raw, "toolCallId");
        const toolName = stringField(raw, "toolName");
        if (toolCallId === undefined || toolName === undefined) return Effect.void;
        return emitTurnStarted().pipe(
          Effect.andThen(
            pushEvent(
              withTurn({
                type: "item.completed",
                itemId: RuntimeItemId.make(toolCallId),
                payload: {
                  itemType: itemTypeForTool(toolName),
                  status: raw.isError === true ? "failed" : "completed",
                  ...(raw.result === undefined ? {} : { data: boundedToolData(raw.result) }),
                },
              }),
            ),
          ),
        );
      }
      case "message_end": {
        const message = parseAssistantMessage(raw.message);
        if (message === undefined) return Effect.void;
        lastAssistantMessage = message;
        return emitUsage(message.usage);
      }
      case "turn_end": {
        const message = parseAssistantMessage(raw.message);
        if (message !== undefined) lastAssistantMessage = message;
        return Effect.void;
      }
      case "agent_settled":
        return settleTurn();
      case "extension_ui_request": {
        const id = stringField(raw, "id");
        const method = stringField(raw, "method");
        if (
          id === undefined ||
          (method !== "select" && method !== "confirm" && method !== "input" && method !== "editor")
        ) {
          return Effect.void;
        }
        uiRequests.set(id, method);
        const rawOptions = method === "confirm" ? ["Yes", "No"] : raw.options;
        const options = Array.isArray(rawOptions) ? rawOptions.map((option) => String(option)) : [];
        const title = stringField(raw, "title") || method;
        const question = stringField(raw, "message") || title;
        return pushEvent(
          withTurn({
            type: "user-input.requested",
            requestId: RuntimeRequestId.make(id),
            payload: {
              questions: [
                {
                  id,
                  header: title,
                  question,
                  options: options.map((option) => ({ label: option, description: option })),
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

  const failPending = Effect.gen(function* () {
    transportClosed = true;
    for (const reply of pending.values()) {
      yield* Deferred.fail(
        reply,
        adapterError("pi-rpc", "Pi RPC output closed before the command completed."),
      );
    }
    pending.clear();
  });

  const pump = child.stdout.pipe(
    Stream.decodeText(),
    Stream.mapEffect((chunk) =>
      Effect.gen(function* () {
        buffer += chunk;
        const lines: Array<string> = [];
        for (;;) {
          const index = buffer.indexOf("\n");
          if (index < 0) break;
          lines.push(buffer.slice(0, index).replace(/\r$/, ""));
          buffer = buffer.slice(index + 1);
        }
        if (buffer.length > 2_000_000 || lines.some((line) => line.length > 2_000_000)) {
          yield* child.close.pipe(Effect.ignore);
          return yield* adapterError("pi-rpc", "Pi RPC output exceeded the 2 MB line limit.");
        }
        return lines;
      }),
    ),
    Stream.flatMap(Stream.fromIterable),
    Stream.mapEffect((line) =>
      Effect.gen(function* () {
        const decoded = decodeJsonExit(line);
        if (Exit.isFailure(decoded)) return;
        const message = decoded.value;
        if (isRecord(message) && message.type === "response") {
          const id = stringField(message, "id");
          const reply = id === undefined ? undefined : pending.get(id);
          if (reply !== undefined) {
            pending.delete(id!);
            if (message.success === false) {
              yield* Deferred.fail(
                reply,
                adapterError(
                  stringField(message, "command") ?? "pi-command",
                  stringField(message, "error") ?? "Pi command failed",
                ),
              );
            } else {
              yield* Deferred.succeed(reply, message.data ?? {});
            }
          }
          return;
        }
        yield* handlePiEvent(message);
      }),
    ),
    Stream.runDrain,
    Effect.ensuring(failPending),
  );

  const request = (command: Readonly<Record<string, unknown>>) =>
    Effect.gen(function* () {
      if (transportClosed) {
        return yield* adapterError(String(command.type), "Pi RPC output is closed.");
      }
      requestSequence += 1;
      const id = `t3-${requestSequence}`;
      const reply = yield* Deferred.make<unknown, ProviderAdapterV1Error>();
      pending.set(id, reply);
      const payload = yield* encodeJson({ id, ...command }).pipe(
        Effect.mapError((cause) =>
          adapterError(String(command.type), "Pi command is not JSON-serializable.", cause),
        ),
      );
      const result = yield* child
        .write(new TextEncoder().encode(`${payload}\n`))
        .pipe(Effect.result);
      if (Result.isFailure(result)) {
        pending.delete(id);
        return yield* adapterError(
          String(command.type),
          `Could not write to Pi process: ${result.failure.detail}`,
          result.failure,
        );
      }
      return yield* Deferred.await(reply).pipe(
        Effect.timeout(command.type === "abort" ? "2 minutes" : "30 seconds"),
        Effect.mapError((cause) =>
          isAdapterError(cause)
            ? cause
            : adapterError(
                String(command.type),
                `Timed out waiting for Pi RPC command '${String(command.type)}'.`,
                cause,
              ),
        ),
        Effect.ensuring(Effect.sync(() => pending.delete(id))),
      );
    });

  const notify = (command: Readonly<Record<string, unknown>>) =>
    encodeJson(command).pipe(
      Effect.mapError((cause) =>
        adapterError(String(command.type), "Pi response is not JSON-serializable.", cause),
      ),
      Effect.flatMap((payload) => child.write(new TextEncoder().encode(`${payload}\n`))),
      Effect.mapError((cause) =>
        isAdapterError(cause)
          ? cause
          : adapterError(
              String(command.type),
              `Could not write to Pi process: ${cause.detail}`,
              cause,
            ),
      ),
    );

  return {
    threadId: input.threadId,
    request,
    notify,
    startPump: Effect.all([pump, child.stderr.pipe(Stream.runDrain)], {
      concurrency: "unbounded",
      discard: true,
    }).pipe(Effect.forkIn(input.scope), Effect.asVoid),
    exitCode: child.exitCode,
    expectExit: child.expectExit,
    close: child.close,
    scope: input.scope,
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
    clearAbortRequested: () => {
      abortRequested = false;
    },
    get activeTurnId() {
      return activeTurnId;
    },
    get isClosed() {
      return transportClosed;
    },
  };
}

/**
 * Map one Pi catalog entry onto the identity T3 shows for a model.
 *
 * Pi lists one entry per logged-in account and names the account with a
 * suffixed provider id (`openai-codex`, `openai-codex-3`, `openrouter`). Two
 * accounts therefore offer the same model under the same name, so `provider`
 * is the only field that tells those rows apart. Surface it as `subProvider`,
 * the same field the Prime, OpenCode, and ACP adapters populate, so the picker
 * can label the route instead of repeating the instance name.
 */
export function mapPiModelIdentity(model: {
  readonly provider: string;
  readonly id: string;
  readonly name?: string | undefined;
}): { readonly slug: string; readonly name: string; readonly subProvider?: string } {
  const provider = model.provider.trim();
  return {
    slug: `${model.provider}/${model.id}`,
    name: model.name ?? model.id,
    ...(provider.length > 0 ? { subProvider: provider } : {}),
  };
}

function parseQualifiedModel(
  model: string,
): { readonly provider: string; readonly modelId: string } | undefined {
  const separator = model.indexOf("/");
  if (separator < 1 || separator === model.length - 1) return undefined;
  return { provider: model.slice(0, separator), modelId: model.slice(separator + 1) };
}

export function makePiAdapter(
  config: PiProviderAdapterConfig,
  input: ProviderAdapterCreateInputV1<PiProviderAdapterConfig>,
  host: ProviderAdapterHostV2,
) {
  return Effect.gen(function* () {
    const forbiddenFlag = forbiddenLaunchFlag(config.args);
    if (forbiddenFlag !== undefined) {
      return yield* adapterError(
        "configure",
        `Pi launch arguments must not set '${forbiddenFlag}'; T3 owns RPC mode and session selection.`,
      );
    }

    const adapterScope = yield* Effect.scope;
    const events = yield* PubSub.unbounded<ProviderRuntimeEvent>();
    const sessionMutation = yield* Semaphore.make(1);
    const sessions = new Map<ThreadId, PiSessionEntry>();

    const spawnConnection = (threadId: ThreadId, requestedCwd?: string) =>
      Effect.gen(function* () {
        const scope = yield* Scope.make();
        return yield* Effect.gen(function* () {
          const storage = yield* host.storage
            .prepareSession(threadId)
            .pipe(Effect.mapError((cause) => adapterError("startSession", cause.detail, cause)));
          const cwd = yield* host.workspaces
            .resolveCwd(requestedCwd ?? config.cwd)
            .pipe(Effect.mapError((cause) => adapterError("startSession", cause.detail, cause)));
          const spawned = yield* host.processes
            .spawn({
              command: config.binaryPath,
              args: launchArgs(config.args, storage.sessionDirectory),
              cwd,
              environment: input.environment,
              purpose: { kind: "session", threadId },
            })
            .pipe(
              Effect.provideService(Scope.Scope, scope),
              Effect.mapError((cause) =>
                adapterError("startSession", `Could not start Pi process: ${cause.detail}`, cause),
              ),
            );
          const connection = makeConnection({
            process: spawned,
            providerInstanceId: input.instanceId,
            threadId,
            events,
            scope,
          });
          yield* connection.startPump;
          return connection;
        }).pipe(Effect.onError(() => Scope.close(scope, Exit.void)));
      });

    const applyModelSelection = (connection: PiConnection, selection: ModelSelection | undefined) =>
      Effect.gen(function* () {
        if (selection === undefined) return;
        const parsed = parseQualifiedModel(selection.model);
        if (parsed === undefined) {
          return yield* adapterError(
            "set_model",
            "Pi models must use a provider/modelId selection.",
          );
        }
        yield* connection.request({ type: "set_model", ...parsed });
        for (const option of selection.options ?? []) {
          if (option.id === "reasoningEffort") {
            yield* connection.request({ type: "set_thinking_level", level: String(option.value) });
          }
        }
      });

    const buildSession = (
      threadId: ThreadId,
      runtimeMode: ProviderSession["runtimeMode"],
      cwd: string,
      resumeCursor: PiResumeCursor,
      timestamp: string,
    ): ProviderSession => ({
      provider: PROVIDER,
      providerInstanceId: input.instanceId,
      status: "ready",
      runtimeMode,
      cwd,
      threadId,
      resumeCursor,
      createdAt: timestamp,
      updatedAt: timestamp,
    });

    const startSession: ProviderAdapterV1["startSession"] = (sessionInput) =>
      sessionMutation.withPermits(1)(
        Effect.gen(function* () {
          if (sessionInput.runtimeMode !== "full-access") {
            return yield* adapterError(
              "startSession",
              "Pi currently supports only the full-access runtime mode.",
            );
          }
          const existing = sessions.get(sessionInput.threadId);
          if (existing !== undefined && !existing.connection.isClosed) return existing.session;
          if (existing !== undefined) sessions.delete(sessionInput.threadId);
          const resolvedCwd = yield* host.workspaces
            .resolveCwd(sessionInput.cwd ?? config.cwd)
            .pipe(Effect.mapError((cause) => adapterError("startSession", cause.detail, cause)));
          const connection = yield* spawnConnection(sessionInput.threadId, resolvedCwd);
          return yield* Effect.gen(function* () {
            const requestedResume = parseResumeCursor(sessionInput.resumeCursor);
            if (sessionInput.resumeCursor !== undefined && requestedResume === undefined) {
              return yield* adapterError(
                "startSession",
                "Pi resume requires a native sessionFile and sessionId.",
              );
            }
            if (requestedResume !== undefined) {
              const sessionFile = yield* host.storage
                .validateSessionFile({
                  threadId: sessionInput.threadId,
                  path: requestedResume.sessionFile,
                  mustExist: true,
                })
                .pipe(
                  Effect.mapError((cause) => adapterError("startSession", cause.detail, cause)),
                );
              const switched = yield* connection.request({
                type: "switch_session",
                sessionPath: sessionFile,
              });
              const switchResult = yield* decodeSwitchSession(switched).pipe(
                Effect.mapError((cause) =>
                  adapterError("startSession", "Invalid Pi resume response.", cause),
                ),
              );
              if (switchResult.cancelled === true) {
                return yield* adapterError("startSession", "Pi session resume was cancelled.");
              }
            }
            yield* applyModelSelection(
              connection,
              sessionInput.modelSelection ??
                (config.model === undefined
                  ? undefined
                  : { instanceId: input.instanceId, model: config.model }),
            );
            if (config.thinkingLevel !== undefined && sessionInput.modelSelection === undefined) {
              yield* connection.request({
                type: "set_thinking_level",
                level: config.thinkingLevel,
              });
            }
            const rawState = yield* connection.request({ type: "get_state" });
            const state = yield* decodeState(rawState).pipe(
              Effect.mapError((cause) =>
                adapterError("startSession", "Invalid Pi state response.", cause),
              ),
            );
            if (requestedResume !== undefined && state.sessionId !== requestedResume.sessionId) {
              return yield* adapterError(
                "startSession",
                `Pi resumed session '${state.sessionId}', expected '${requestedResume.sessionId}'.`,
              );
            }
            const sessionFile = yield* host.storage
              .validateSessionFile({
                threadId: sessionInput.threadId,
                path: state.sessionFile,
                mustExist: requestedResume !== undefined,
              })
              .pipe(Effect.mapError((cause) => adapterError("startSession", cause.detail, cause)));
            const cursor = { sessionFile, sessionId: state.sessionId };
            const timestamp = yield* nowIso;
            const session = buildSession(
              sessionInput.threadId,
              sessionInput.runtimeMode,
              resolvedCwd,
              cursor,
              timestamp,
            );
            const entry = { connection, session, resumeCursor: cursor };
            sessions.set(sessionInput.threadId, entry);
            yield* connection.exitCode.pipe(
              Effect.ensuring(
                Effect.sync(() => {
                  if (sessions.get(sessionInput.threadId) === entry) {
                    sessions.delete(sessionInput.threadId);
                  }
                }).pipe(Effect.andThen(Scope.close(connection.scope, Exit.void)), Effect.ignore),
              ),
              Effect.forkIn(adapterScope),
            );
            return session;
          }).pipe(
            Effect.onError(() =>
              Effect.all([connection.close, Scope.close(connection.scope, Exit.void)], {
                discard: true,
              }).pipe(Effect.ignore),
            ),
          );
        }),
      );

    const requireEntry = (threadId: ThreadId, operation: string) => {
      const entry = sessions.get(threadId);
      if (entry === undefined || entry.connection.isClosed) {
        if (entry !== undefined) sessions.delete(threadId);
        return Effect.fail(adapterError(operation, `No Pi session for thread '${threadId}'.`));
      }
      return Effect.succeed(entry);
    };

    const runPrompt = (entry: PiSessionEntry, command: Readonly<Record<string, unknown>>) =>
      Effect.gen(function* () {
        const turnId = TurnId.make(`pi-turn-${yield* Effect.sync(NodeCrypto.randomUUID)}`);
        entry.connection.beginTurn(turnId);
        const accepted = yield* entry.connection.request(command).pipe(Effect.result);
        if (Result.isFailure(accepted)) {
          entry.connection.resetTurn();
          return yield* accepted.failure;
        }
        const state = yield* entry.connection.request({ type: "get_state" }).pipe(
          Effect.flatMap(decodeState),
          Effect.mapError((cause) =>
            isAdapterError(cause)
              ? cause
              : adapterError("get_state", "Invalid Pi state response.", cause),
          ),
        );
        yield* entry.connection.settleHandledPrompt(turnId, state);
        return {
          threadId: entry.session.threadId,
          turnId,
          resumeCursor: entry.resumeCursor,
        };
      }).pipe(Effect.onError(() => Effect.sync(entry.connection.resetTurn)));

    const stopEntry = (threadId: ThreadId) =>
      Effect.gen(function* () {
        const entry = sessions.get(threadId);
        if (entry === undefined) return;
        sessions.delete(threadId);
        yield* entry.connection.expectExit;
        yield* entry.connection.close;
        yield* Scope.close(entry.connection.scope, Exit.void).pipe(Effect.ignore);
        yield* entry.connection.exitCode.pipe(Effect.ignore);
      });

    yield* Effect.addFinalizer(() =>
      Effect.gen(function* () {
        yield* Effect.forEach(Array.from(sessions.keys()), stopEntry, { discard: true }).pipe(
          Effect.ignore,
        );
        yield* PubSub.shutdown(events);
      }),
    );

    const getSnapshot = Effect.gen(function* () {
      if (!input.enabled) {
        return {
          instanceId: input.instanceId,
          driver: PROVIDER,
          displayName: input.displayName,
          enabled: false,
          installed: false,
          version: null,
          status: "disabled",
          message: "Pi is disabled for this provider instance.",
          auth: { status: "unknown" },
          checkedAt: yield* nowIso,
          availability: "unavailable",
          supportedRuntimeModes: ["full-access"],
          models: [],
          slashCommands: [],
          skills: [],
        } satisfies ServerProvider;
      }
      const scope = yield* Scope.make();
      return yield* Effect.gen(function* () {
        const storage = yield* host.storage
          .prepareSession(PROBE_THREAD_ID)
          .pipe(Effect.mapError((cause) => adapterError("probe", cause.detail, cause)));
        const cwd = yield* host.workspaces
          .resolveCwd(config.cwd)
          .pipe(Effect.mapError((cause) => adapterError("probe", cause.detail, cause)));
        const probe = yield* host.processes
          .spawn({
            command: config.binaryPath,
            args: launchArgs(config.args, storage.sessionDirectory),
            cwd,
            environment: input.environment,
            purpose: { kind: "probe" },
          })
          .pipe(Effect.provideService(Scope.Scope, scope), Effect.result);
        if (Result.isFailure(probe)) {
          return yield* adapterError("probe", probe.failure.detail, probe.failure);
        }
        let models: ServerProvider["models"] = [];
        {
          const connection = makeConnection({
            process: probe.success,
            providerInstanceId: input.instanceId,
            threadId: PROBE_THREAD_ID,
            events,
            scope,
          });
          yield* connection.startPump;
          const discovered = yield* connection.request({ type: "get_available_models" }).pipe(
            Effect.flatMap(decodeModels),
            Effect.mapError((cause) => adapterError("probe", "Invalid Pi model response.", cause)),
          );
          models = yield* Effect.forEach(
            discovered.models,
            (model) =>
              Effect.gen(function* () {
                yield* connection.request({
                  type: "set_model",
                  provider: model.provider,
                  modelId: model.id,
                });
                const thinkingLevels = yield* connection
                  .request({ type: "get_available_thinking_levels" })
                  .pipe(
                    Effect.flatMap(decodeThinkingLevels),
                    Effect.mapError((cause) =>
                      adapterError("probe", "Invalid Pi thinking-level response.", cause),
                    ),
                  );
                const state = yield* connection.request({ type: "get_state" }).pipe(
                  Effect.flatMap(decodeState),
                  Effect.mapError((cause) =>
                    adapterError("probe", "Invalid Pi state response.", cause),
                  ),
                );
                const optionDescriptors =
                  thinkingLevels.levels.length === 0
                    ? []
                    : [
                        {
                          id: "reasoningEffort",
                          label: "Reasoning",
                          type: "select" as const,
                          options: thinkingLevels.levels.map((level) => ({
                            id: level,
                            label: level,
                            ...(level === state.thinkingLevel ? { isDefault: true as const } : {}),
                          })),
                          ...(state.thinkingLevel === undefined
                            ? {}
                            : { currentValue: state.thinkingLevel }),
                        },
                      ];
                return {
                  ...mapPiModelIdentity(model),
                  isCustom: false,
                  capabilities: { optionDescriptors },
                };
              }),
            { concurrency: 1 },
          );
          yield* probe.success.expectExit;
          yield* probe.success.close;
          yield* probe.success.exitCode.pipe(Effect.ignore);
        }
        const checkedAt = yield* nowIso;
        return {
          instanceId: input.instanceId,
          driver: PROVIDER,
          displayName: input.displayName,
          enabled: input.enabled,
          installed: true,
          version: "1.0.0",
          status: input.enabled ? "ready" : "disabled",
          auth: { status: "unknown" },
          checkedAt,
          availability: "available",
          supportedRuntimeModes: ["full-access"],
          models,
          slashCommands: [],
          skills: [],
        } satisfies ServerProvider;
      }).pipe(Effect.ensuring(Scope.close(scope, Exit.void)));
    });

    const safeSnapshot = getSnapshot.pipe(
      Effect.catch((cause) =>
        Effect.gen(function* () {
          const missing = commandAppearsMissing(cause);
          return {
            instanceId: input.instanceId,
            driver: PROVIDER,
            displayName: input.displayName,
            enabled: input.enabled,
            installed: !missing,
            version: "1.0.0",
            status: "error",
            message: missing
              ? `Pi (\`${config.binaryPath}\`) is not installed or not on PATH.`
              : "Failed to query the Pi RPC provider.",
            auth: { status: "unknown" },
            checkedAt: yield* nowIso,
            availability: "unavailable",
            supportedRuntimeModes: ["full-access"],
            models: [],
            slashCommands: [],
            skills: [],
          } satisfies ServerProvider;
        }),
      ),
    );

    const adapter: ProviderAdapterV1 = {
      capabilities: PI_PROVIDER_ADAPTER_CAPABILITIES,
      startSession,
      sendTurn: (turnInput) =>
        Effect.gen(function* () {
          if ((turnInput.attachments?.length ?? 0) > 0) {
            return yield* adapterError("sendTurn", "Pi attachment input is not supported.");
          }
          const message = turnInput.input;
          if (!message?.trim()) {
            return yield* adapterError("sendTurn", "Pi requires a non-empty text prompt.");
          }
          const entry = yield* requireEntry(turnInput.threadId, "sendTurn");
          const activeTurnId = entry.connection.activeTurnId;
          if (activeTurnId !== undefined) {
            yield* entry.connection.request({ type: "steer", message });
            return {
              threadId: turnInput.threadId,
              turnId: activeTurnId,
              resumeCursor: entry.resumeCursor,
            };
          }
          if (turnInput.modelSelection !== undefined) {
            yield* applyModelSelection(entry.connection, turnInput.modelSelection);
          }
          return yield* runPrompt(entry, { type: "prompt", message });
        }),
      interruptTurn: (threadId) =>
        Effect.gen(function* () {
          const entry = yield* requireEntry(threadId, "interruptTurn");
          entry.connection.markAbortRequested();
          yield* entry.connection
            .request({ type: "abort" })
            .pipe(Effect.tapError(() => Effect.sync(entry.connection.clearAbortRequested)));
        }),
      respondToRequest: () =>
        Effect.fail(
          adapterError(
            "respondToRequest",
            "Pi approvals require a controlled extension and are not negotiated.",
          ),
        ),
      respondToUserInput: (threadId, requestId, answers: ProviderUserInputAnswers) =>
        Effect.gen(function* () {
          const entry = yield* requireEntry(threadId, "respondToUserInput");
          const method = entry.connection.uiRequests.get(requestId);
          if (method === undefined) {
            return yield* adapterError("respondToUserInput", "Unknown Pi UI request.");
          }
          const answer = answers[requestId];
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
          yield* entry.connection.notify(response);
          entry.connection.uiRequests.delete(requestId);
          yield* PubSub.publish(events, {
            eventId: EventId.make(yield* Effect.sync(NodeCrypto.randomUUID)),
            provider: PROVIDER,
            providerInstanceId: input.instanceId,
            threadId,
            createdAt: yield* nowIso,
            type: "user-input.resolved",
            requestId: RuntimeRequestId.make(requestId),
            payload: { answers },
          });
        }),
      stopSession: (threadId) => sessionMutation.withPermits(1)(stopEntry(threadId)),
      listSessions: () =>
        Effect.sync(() => Array.from(sessions.values(), (entry) => entry.session)),
      hasSession: (threadId) =>
        Effect.sync(() => {
          const entry = sessions.get(threadId);
          if (entry?.connection.isClosed) sessions.delete(threadId);
          return entry !== undefined && !entry.connection.isClosed;
        }),
      readThread: (threadId) =>
        Effect.gen(function* () {
          const entry = yield* requireEntry(threadId, "readThread");
          const raw = yield* entry.connection.request({ type: "get_entries" });
          const result = yield* decodeEntries(raw).pipe(
            Effect.mapError((cause) =>
              adapterError("readThread", "Invalid Pi entries response.", cause),
            ),
          );
          const byId = new Map(result.entries.map((item) => [item.id, item]));
          const branch: Array<(typeof result.entries)[number]> = [];
          const visited = new Set<string>();
          let cursor = result.leafId;
          while (cursor != null && !visited.has(cursor)) {
            visited.add(cursor);
            const item = byId.get(cursor);
            if (item === undefined) break;
            branch.push(item);
            cursor = item.parentId;
          }
          return {
            threadId,
            turns: branch
              .toReversed()
              .filter((item) => item.type === "message" && item.message !== undefined)
              .map((item) => ({ id: TurnId.make(item.id), items: [item.message] })),
          } satisfies ProviderAdapterThreadSnapshotV1;
        }),
      rollbackThread: () =>
        Effect.fail(
          adapterError(
            "rollbackThread",
            "Pi rollback forks sessions and is not negotiated for protocol 1.",
          ),
        ),
      stopAll: () =>
        sessionMutation.withPermits(1)(
          Effect.forEach(Array.from(sessions.keys()), stopEntry).pipe(Effect.asVoid),
        ),
      streamEvents: Stream.fromPubSub(events),
    };

    return {
      snapshot: {
        getSnapshot: safeSnapshot,
        refresh: safeSnapshot,
        streamChanges: Stream.empty,
      },
      adapter,
    };
  });
}
