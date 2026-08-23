// @effect-diagnostics nodeBuiltinImport:off
import {
  EventId,
  type ModelSelection,
  type PrimeSettings,
  type ProviderApprovalDecision,
  ProviderDriverKind,
  type ProviderRuntimeEvent,
  type ProviderSendTurnInput,
  type ProviderSession,
  ProviderInstanceId,
  RuntimeItemId,
  RuntimeRequestId,
  RuntimeTaskId,
  type ThreadId,
  type ToolLifecycleItemType,
  TurnId,
} from "@t3tools/contracts";
import { tokenizeCliArgs } from "@t3tools/shared/cliArgs";
import { getModelSelectionStringOptionValue } from "@t3tools/shared/model";
import * as Crypto from "effect/Crypto";
import * as DateTime from "effect/DateTime";
import * as Effect from "effect/Effect";
import * as Exit from "effect/Exit";
import * as FileSystem from "effect/FileSystem";
import * as Path from "effect/Path";
import * as PubSub from "effect/PubSub";
import * as Queue from "effect/Queue";
import * as Schema from "effect/Schema";
import * as Scope from "effect/Scope";
import * as Stream from "effect/Stream";
import * as ChildProcessSpawner from "effect/unstable/process/ChildProcessSpawner";

import { resolveAttachmentPath } from "../../attachmentStore.ts";
import { ServerConfig } from "../../config.ts";
import {
  ProviderAdapterRequestError,
  ProviderAdapterSessionNotFoundError,
  ProviderAdapterValidationError,
} from "../Errors.ts";
import { spawnPrimeRpcClient, type PrimeRpcClient } from "../prime/PrimeRpcClient.ts";
import {
  isPrimeApprovalExtensionHandshake,
  preparePrimeApprovalExtension,
  PRIME_APPROVAL_EXTENSION_MODE_FLAG,
} from "../prime/primeApprovalExtension.ts";
import { preparePrimeOpenRouterCatalogExtension } from "../prime/primeOpenRouterCatalogExtension.ts";
import {
  parsePrimeModelSlug,
  primeModelSlug,
  PRIME_THINKING_LEVEL_OPTION_ID,
} from "../prime/primeModels.ts";
import { type PrimeAdapterShape } from "../Services/PrimeAdapter.ts";

const PROVIDER = ProviderDriverKind.make("primeAgent");
const PRIME_RESUME_VERSION = 1 as const;

const PrimeState = Schema.Struct({
  sessionFile: Schema.String,
  sessionId: Schema.String,
  isStreaming: Schema.Boolean,
  thinkingLevel: Schema.optional(Schema.String),
  model: Schema.optional(
    Schema.NullOr(
      Schema.Struct({
        id: Schema.String,
        provider: Schema.optional(Schema.String),
      }),
    ),
  ),
});

const PrimeAgentStart = Schema.Struct({ type: Schema.Literal("agent_start") });
const PrimeTextDelta = Schema.Struct({
  type: Schema.Literal("message_update"),
  assistantMessageEvent: Schema.Struct({
    type: Schema.Literal("text_delta"),
    delta: Schema.String,
    contentIndex: Schema.optional(Schema.Int),
  }),
});
const PrimeThinkingDelta = Schema.Struct({
  type: Schema.Literal("message_update"),
  assistantMessageEvent: Schema.Struct({
    type: Schema.Literal("thinking_delta"),
    delta: Schema.String,
    contentIndex: Schema.optional(Schema.Int),
  }),
});
const PrimeToolExecutionStart = Schema.Struct({
  type: Schema.Literal("tool_execution_start"),
  toolCallId: Schema.String,
  toolName: Schema.String,
  args: Schema.optional(Schema.Unknown),
});
const PrimeToolExecutionUpdate = Schema.Struct({
  type: Schema.Literal("tool_execution_update"),
  toolCallId: Schema.String,
  toolName: Schema.String,
  args: Schema.optional(Schema.Unknown),
  partialResult: Schema.optional(Schema.Unknown),
});
const PrimeToolExecutionEnd = Schema.Struct({
  type: Schema.Literal("tool_execution_end"),
  toolCallId: Schema.String,
  toolName: Schema.String,
  result: Schema.optional(Schema.Unknown),
  isError: Schema.optional(Schema.Boolean),
});
const PrimeUsageMessage = Schema.Struct({
  usage: Schema.Struct({
    input: Schema.Number,
    output: Schema.Number,
    cacheRead: Schema.optional(Schema.Number),
    cacheWrite: Schema.optional(Schema.Number),
    totalTokens: Schema.Number,
  }),
});
const PrimeAssistantOutcome = Schema.Struct({
  role: Schema.Literal("assistant"),
  stopReason: Schema.optional(Schema.NullOr(Schema.String)),
  errorMessage: Schema.optional(Schema.String),
});
const PrimeAgentEnd = Schema.Struct({
  type: Schema.Literal("agent_end"),
  messages: Schema.optional(Schema.Array(Schema.Unknown)),
});
const PrimeExtensionConfirmRequest = Schema.Struct({
  type: Schema.Literal("extension_ui_request"),
  id: Schema.String,
  method: Schema.Literal("confirm"),
  title: Schema.String,
  message: Schema.String,
  timeout: Schema.optional(Schema.Number),
});
const PrimeExtensionSelectRequest = Schema.Struct({
  type: Schema.Literal("extension_ui_request"),
  id: Schema.String,
  method: Schema.Literal("select"),
  title: Schema.String,
  options: Schema.Array(Schema.String),
  timeout: Schema.optional(Schema.Number),
});
const PrimeExtensionInputRequest = Schema.Struct({
  type: Schema.Literal("extension_ui_request"),
  id: Schema.String,
  method: Schema.Literal("input"),
  title: Schema.String,
  placeholder: Schema.optional(Schema.String),
  timeout: Schema.optional(Schema.Number),
});
const PrimeExtensionEditorRequest = Schema.Struct({
  type: Schema.Literal("extension_ui_request"),
  id: Schema.String,
  method: Schema.Literal("editor"),
  title: Schema.String,
  prefill: Schema.optional(Schema.String),
});
const PrimeRlmChildUpdate = Schema.Struct({
  type: Schema.Literal("rlm_child_update"),
  child: Schema.Struct({
    id: Schema.String,
    sessionName: Schema.optional(Schema.String),
    model: Schema.optional(Schema.String),
    label: Schema.String,
    status: Schema.Literals(["queued", "running", "done", "error", "cancelled"]),
    answerPreview: Schema.optional(Schema.String),
    recap: Schema.optional(Schema.String),
    activity: Schema.optional(
      Schema.Struct({
        kind: Schema.Literals(["waiting", "writing", "executing"]),
        toolName: Schema.optional(Schema.String),
      }),
    ),
    error: Schema.optional(Schema.String),
  }),
});

const decodeRlmChildUpdate = Schema.decodeUnknownExit(PrimeRlmChildUpdate);

const nonEmptyTrimmed = (value: string | undefined): string | undefined => {
  const trimmed = value?.trim();
  return trimmed ? trimmed : undefined;
};

function primeChildProgressSummary(child: (typeof PrimeRlmChildUpdate.Type)["child"]): string {
  const recap = nonEmptyTrimmed(child.recap);
  if (recap !== undefined) {
    return recap;
  }
  if (child.activity?.kind === "executing") {
    const toolName = nonEmptyTrimmed(child.activity.toolName);
    return toolName !== undefined ? `Using ${toolName}` : "Using a tool";
  }
  if (child.activity?.kind === "writing") {
    return "Writing response";
  }
  if (child.activity?.kind === "waiting") {
    return "Waiting";
  }
  return child.status === "queued" ? "Queued" : "Working";
}

const PrimeExtensionUserInputRequest = Schema.Union([
  PrimeExtensionSelectRequest,
  PrimeExtensionInputRequest,
  PrimeExtensionEditorRequest,
]);

const decodePrimeState = Schema.decodeUnknownEffect(PrimeState);
const decodeAgentStart = Schema.decodeUnknownExit(PrimeAgentStart);
const decodeTextDelta = Schema.decodeUnknownExit(PrimeTextDelta);
const decodeThinkingDelta = Schema.decodeUnknownExit(PrimeThinkingDelta);
const decodeToolExecutionStart = Schema.decodeUnknownExit(PrimeToolExecutionStart);
const decodeToolExecutionUpdate = Schema.decodeUnknownExit(PrimeToolExecutionUpdate);
const decodeToolExecutionEnd = Schema.decodeUnknownExit(PrimeToolExecutionEnd);
const decodeUsageMessage = Schema.decodeUnknownExit(PrimeUsageMessage);
const decodeAssistantOutcome = Schema.decodeUnknownExit(PrimeAssistantOutcome);
const decodeAgentEnd = Schema.decodeUnknownExit(PrimeAgentEnd);
const decodeExtensionConfirmRequest = Schema.decodeUnknownExit(PrimeExtensionConfirmRequest);
const decodeExtensionUserInputRequest = Schema.decodeUnknownExit(PrimeExtensionUserInputRequest);

const nowIso = Effect.map(DateTime.now, DateTime.formatIso);

export interface PrimeAdapterLiveOptions {
  readonly environment?: NodeJS.ProcessEnv;
  readonly instanceId?: ProviderInstanceId;
}

interface PendingPrimeApproval {
  readonly requestType: "command_execution_approval" | "dynamic_tool_call";
  readonly turnId: TurnId;
  readonly detail: string;
  readonly request: typeof PrimeExtensionConfirmRequest.Type;
}

interface PendingPrimeUserInput {
  readonly turnId: TurnId;
  readonly request: typeof PrimeExtensionUserInputRequest.Type;
  readonly options: ReadonlyArray<string>;
}

interface PrimeSubagentState {
  title: string;
  model: string | undefined;
  lastProgressKey: string | undefined;
  terminal: boolean;
}

interface PrimeSessionContext {
  readonly threadId: ThreadId;
  readonly rpc: PrimeRpcClient;
  readonly scope: Scope.Closeable;
  session: ProviderSession;
  currentModelSlug: string | undefined;
  currentThinkingLevel: string | undefined;
  activeTurnId: TurnId | undefined;
  turns: Array<{ id: TurnId; items: Array<unknown> }>;
  latestAgentEndMessages: ReadonlyArray<unknown> | undefined;
  readonly pendingApprovals: Map<string, PendingPrimeApproval>;
  readonly pendingUserInputs: Map<string, PendingPrimeUserInput>;
  turnStarted: boolean;
  stopped: boolean;
  settledTurnId: TurnId | undefined;
  /** Last emitted Prime RLM child state, used to dedupe streaming snapshots. */
  readonly subagents: Map<string, PrimeSubagentState>;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parsePrimeResume(
  raw: unknown,
): { readonly sessionId: string; readonly sessionFile: string } | undefined {
  if (!isRecord(raw)) {
    return undefined;
  }
  if (raw.schemaVersion !== PRIME_RESUME_VERSION) {
    return undefined;
  }
  if (typeof raw.sessionId !== "string" || raw.sessionId.trim().length === 0) {
    return undefined;
  }
  if (typeof raw.sessionFile !== "string" || raw.sessionFile.trim().length === 0) {
    return undefined;
  }
  return {
    sessionId: raw.sessionId.trim(),
    sessionFile: raw.sessionFile.trim(),
  };
}

function isPathInsideDirectory(path: Path.Path, directory: string, file: string): boolean {
  const resolvedDirectory = path.resolve(directory);
  const resolvedFile = path.resolve(file);
  const relative = path.relative(resolvedDirectory, resolvedFile);
  return relative !== "" && !relative.startsWith("..") && !path.isAbsolute(relative);
}

function modelSlugFromPrimeState(state: typeof PrimeState.Type): string | undefined {
  if (!state.model) {
    return undefined;
  }
  const id = state.model.id.trim();
  const provider = state.model.provider?.trim();
  if (provider && id) {
    return primeModelSlug({ provider, id });
  }
  return id.length > 0 ? id : undefined;
}

function toToolLifecycleItemType(toolName: string): ToolLifecycleItemType {
  const normalized = toolName.toLowerCase();
  if (normalized.includes("bash") || normalized.includes("command") || normalized === "ipython") {
    return "command_execution";
  }
  if (normalized.includes("edit") || normalized.includes("write") || normalized.includes("patch")) {
    return "file_change";
  }
  if (normalized.includes("web") || normalized.includes("search")) {
    return "web_search";
  }
  if (normalized.includes("mcp")) {
    return "mcp_tool_call";
  }
  if (normalized.includes("image")) {
    return "image_view";
  }
  return "dynamic_tool_call";
}

function toolResultDetail(value: unknown): string | undefined {
  if (typeof value === "string") {
    const trimmed = value.trim();
    return trimmed.length > 0 ? trimmed : undefined;
  }
  if (isRecord(value) && Array.isArray(value.content)) {
    const text = value.content
      .filter(isRecord)
      .map((part) => (typeof part.text === "string" ? part.text : ""))
      .join("");
    const trimmed = text.trim();
    return trimmed.length > 0 ? trimmed : undefined;
  }
  if (value === undefined) {
    return undefined;
  }
  try {
    const json = JSON.stringify(value);
    return json && json !== "{}" && json !== "null" ? json : undefined;
  } catch {
    return undefined;
  }
}

function lastAssistantOutcome(messages: ReadonlyArray<unknown>) {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const decoded = decodeAssistantOutcome(messages[index]);
    if (Exit.isSuccess(decoded)) {
      return decoded.value;
    }
  }
  return undefined;
}

function primeFailureFromMessages(
  messages: ReadonlyArray<unknown>,
): { readonly stopReason: string; readonly errorMessage: string } | undefined {
  const outcome = lastAssistantOutcome(messages);
  if (outcome === undefined) {
    return undefined;
  }
  const errorMessage = outcome.errorMessage?.trim();
  if (outcome.stopReason === "error" || errorMessage) {
    return {
      stopReason: outcome.stopReason?.trim() || "error",
      errorMessage: errorMessage || "Prime Agent reported an assistant error.",
    };
  }
  return undefined;
}

function isAbortedTurn(messages: ReadonlyArray<unknown>): boolean {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (!isRecord(message) || message.role !== "assistant") {
      continue;
    }
    const stopReason = typeof message.stopReason === "string" ? message.stopReason.trim() : "";
    return stopReason === "aborted" || stopReason === "cancelled";
  }
  return false;
}

function usageFromMessages(messages: ReadonlyArray<unknown>): unknown | undefined {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const decoded = decodeUsageMessage(messages[index]);
    if (Exit.isSuccess(decoded)) {
      return decoded.value.usage;
    }
  }
  return undefined;
}

function toRpcRequestError(method: string, cause: { readonly message: string }) {
  return new ProviderAdapterRequestError({
    provider: PROVIDER,
    method,
    detail: cause.message,
    cause,
  });
}

function isUnsafeSupervisedLaunchArg(value: string): boolean {
  return (
    value === "-e" ||
    value.startsWith("-e=") ||
    value === "-ne" ||
    value.startsWith("-ne=") ||
    value === "--extension" ||
    value.startsWith("--extension=") ||
    value === "--no-extensions" ||
    value.startsWith("--no-extensions=") ||
    value === `--${PRIME_APPROVAL_EXTENSION_MODE_FLAG}` ||
    value.startsWith(`--${PRIME_APPROVAL_EXTENSION_MODE_FLAG}=`)
  );
}

export function makePrimeAdapter(primeSettings: PrimeSettings, options?: PrimeAdapterLiveOptions) {
  return Effect.gen(function* () {
    const boundInstanceId = options?.instanceId ?? ProviderInstanceId.make("primeAgent");
    const crypto = yield* Crypto.Crypto;
    const childProcessSpawner = yield* ChildProcessSpawner.ChildProcessSpawner;
    const fileSystem = yield* FileSystem.FileSystem;
    const path = yield* Path.Path;
    const serverConfig = yield* ServerConfig;
    const sessions = new Map<ThreadId, PrimeSessionContext>();
    const runtimeEvents = yield* PubSub.unbounded<ProviderRuntimeEvent>();
    const randomUUIDv4 = crypto.randomUUIDv4.pipe(Effect.orDie);
    const stamp = Effect.all({
      eventId: randomUUIDv4.pipe(Effect.map(EventId.make)),
      createdAt: nowIso,
    });
    const publish = (event: ProviderRuntimeEvent) => PubSub.publish(runtimeEvents, event);
    const publishTurnInterrupted = (input: {
      readonly threadId: ThreadId;
      readonly turnId: TurnId;
      readonly reason: string;
    }) =>
      Effect.gen(function* () {
        const base = {
          ...(yield* stamp),
          provider: PROVIDER,
          providerInstanceId: boundInstanceId,
          threadId: input.threadId,
          turnId: input.turnId,
        };
        yield* publish({
          type: "turn.aborted",
          ...base,
          payload: { reason: input.reason },
        });
        // Ingestion settles the orchestration turn on `turn.completed`, not
        // `turn.aborted`. Emit both so Stop clears the running state.
        yield* publish({
          type: "turn.completed",
          ...base,
          payload: {
            state: "interrupted",
            stopReason: "interrupted",
          },
        });
      });

    const requireSession = (
      threadId: ThreadId,
    ): Effect.Effect<PrimeSessionContext, ProviderAdapterSessionNotFoundError> => {
      const session = sessions.get(threadId);
      return session === undefined || session.stopped
        ? Effect.fail(new ProviderAdapterSessionNotFoundError({ provider: PROVIDER, threadId }))
        : Effect.succeed(session);
    };

    const resolvePrimeApproval = Effect.fn("PrimeAdapter.resolvePrimeApproval")(function* (
      ctx: PrimeSessionContext,
      requestId: string,
      decision: Exclude<ProviderApprovalDecision, "acceptForSession">,
      options?: { readonly sendResponse?: boolean },
    ) {
      const pending = ctx.pendingApprovals.get(requestId);
      if (pending === undefined) {
        return yield* new ProviderAdapterRequestError({
          provider: PROVIDER,
          method: "respondToRequest",
          detail: `Prime approval request '${requestId}' is not pending.`,
        });
      }
      const response =
        decision === "accept"
          ? { id: requestId, confirmed: true as const }
          : decision === "decline"
            ? { id: requestId, confirmed: false as const }
            : { id: requestId, cancelled: true as const };
      if (options?.sendResponse !== false) {
        yield* ctx.rpc
          .respondToExtensionUi(response)
          .pipe(Effect.mapError((cause) => toRpcRequestError("extension_ui_response", cause)));
      }
      ctx.pendingApprovals.delete(requestId);
      yield* publish({
        type: "request.resolved",
        ...(yield* stamp),
        provider: PROVIDER,
        providerInstanceId: boundInstanceId,
        threadId: ctx.threadId,
        turnId: pending.turnId,
        requestId: RuntimeRequestId.make(requestId),
        providerRefs: { providerRequestId: requestId },
        payload: {
          requestType: pending.requestType,
          decision,
          resolution: { type: "extension_ui_response", ...response },
        },
      });
    });

    const cancelPendingApprovals = (
      ctx: PrimeSessionContext,
      options?: { readonly sendResponse?: boolean },
    ) =>
      Effect.forEach(
        [...ctx.pendingApprovals.keys()],
        (requestId) => resolvePrimeApproval(ctx, requestId, "cancel", options),
        { discard: true },
      );

    const resolvePrimeUserInput = Effect.fn("PrimeAdapter.resolvePrimeUserInput")(function* (
      ctx: PrimeSessionContext,
      requestId: string,
      answer: string | undefined,
      options?: { readonly sendResponse?: boolean },
    ) {
      const pending = ctx.pendingUserInputs.get(requestId);
      if (pending === undefined) {
        return yield* new ProviderAdapterRequestError({
          provider: PROVIDER,
          method: "respondToUserInput",
          detail: `Prime user-input request '${requestId}' is not pending.`,
        });
      }
      const response =
        answer === undefined
          ? { id: requestId, cancelled: true as const }
          : { id: requestId, value: answer };
      if (options?.sendResponse !== false) {
        yield* ctx.rpc
          .respondToExtensionUi(response)
          .pipe(Effect.mapError((cause) => toRpcRequestError("extension_ui_response", cause)));
      }
      ctx.pendingUserInputs.delete(requestId);
      yield* publish({
        type: "user-input.resolved",
        ...(yield* stamp),
        provider: PROVIDER,
        providerInstanceId: boundInstanceId,
        threadId: ctx.threadId,
        turnId: pending.turnId,
        requestId: RuntimeRequestId.make(requestId),
        providerRefs: { providerRequestId: requestId },
        payload: { answers: answer === undefined ? {} : { [requestId]: answer } },
      });
    });

    const cancelPendingUserInputs = (
      ctx: PrimeSessionContext,
      options?: { readonly sendResponse?: boolean },
    ) =>
      Effect.forEach(
        [...ctx.pendingUserInputs.keys()],
        (requestId) => resolvePrimeUserInput(ctx, requestId, undefined, options),
        { discard: true },
      );

    const cancelPendingRequests = (
      ctx: PrimeSessionContext,
      options?: { readonly sendResponse?: boolean },
    ) =>
      Effect.all([cancelPendingApprovals(ctx, options), cancelPendingUserInputs(ctx, options)], {
        discard: true,
        concurrency: 1,
      });

    const requireSessionFileInsideThreadDir = (input: {
      readonly threadSessionDir: string;
      readonly sessionFile: string;
      readonly detail: string;
    }) =>
      Effect.gen(function* () {
        if (!isPathInsideDirectory(path, input.threadSessionDir, input.sessionFile)) {
          return yield* new ProviderAdapterRequestError({
            provider: PROVIDER,
            method: "resume",
            detail: input.detail,
          });
        }
      });

    const requirePersistedResumeFile = (input: {
      readonly threadSessionDir: string;
      readonly sessionFile: string;
    }) =>
      Effect.gen(function* () {
        yield* requireSessionFileInsideThreadDir({
          threadSessionDir: input.threadSessionDir,
          sessionFile: input.sessionFile,
          detail: "Prime session file is outside this thread's session directory.",
        });
        const exists = yield* fileSystem.exists(input.sessionFile).pipe(
          Effect.mapError(
            (cause) =>
              new ProviderAdapterRequestError({
                provider: PROVIDER,
                method: "resume",
                detail: "Could not inspect the Prime session file.",
                cause,
              }),
          ),
        );
        if (!exists) {
          return yield* new ProviderAdapterRequestError({
            provider: PROVIDER,
            method: "resume",
            detail: "Prime session file is missing; cannot resume.",
          });
        }
        const realDirectory = yield* fileSystem
          .realPath(input.threadSessionDir)
          .pipe(Effect.orElseSucceed(() => path.resolve(input.threadSessionDir)));
        const realFile = yield* fileSystem.realPath(input.sessionFile).pipe(
          Effect.mapError(
            (cause) =>
              new ProviderAdapterRequestError({
                provider: PROVIDER,
                method: "resume",
                detail: "Prime session file is missing; cannot resume.",
                cause,
              }),
          ),
        );
        if (!isPathInsideDirectory(path, realDirectory, realFile)) {
          return yield* new ProviderAdapterRequestError({
            provider: PROVIDER,
            method: "resume",
            detail: "Prime session file is outside this thread's session directory.",
          });
        }
      });

    const applyModelSelection = (input: {
      readonly rpc: PrimeRpcClient;
      readonly modelSelection: ModelSelection | undefined;
      readonly currentModelSlug: string | undefined;
      readonly currentThinkingLevel: string | undefined;
    }) =>
      Effect.gen(function* () {
        let slug = input.currentModelSlug;
        let thinking = input.currentThinkingLevel;
        const requested = input.modelSelection;
        if (requested === undefined || requested.instanceId !== boundInstanceId) {
          return { slug, thinking };
        }
        const parsed = parsePrimeModelSlug(requested.model);
        if (parsed === undefined) {
          return yield* new ProviderAdapterValidationError({
            provider: PROVIDER,
            operation: "set_model",
            issue: `Prime model '${requested.model}' must be a provider/id slug.`,
          });
        }
        const nextSlug = primeModelSlug({ provider: parsed.provider, id: parsed.modelId });
        if (nextSlug !== slug) {
          yield* input.rpc
            .request({
              type: "set_model",
              provider: parsed.provider,
              modelId: parsed.modelId,
            })
            .pipe(Effect.mapError((cause) => toRpcRequestError("set_model", cause)));
          slug = nextSlug;
        }
        const nextThinking = getModelSelectionStringOptionValue(
          requested,
          PRIME_THINKING_LEVEL_OPTION_ID,
        );
        if (nextThinking !== undefined && nextThinking !== thinking) {
          yield* input.rpc
            .request({
              type: "set_thinking_level",
              level: nextThinking,
            })
            .pipe(Effect.mapError((cause) => toRpcRequestError("set_thinking_level", cause)));
          thinking = nextThinking;
        }
        return { slug, thinking };
      });

    const requireApprovalExtensionHandshake = Effect.fn(
      "PrimeAdapter.requireApprovalExtensionHandshake",
    )(function* (rpc: PrimeRpcClient, extensionPath: string) {
      const response = yield* rpc.request({ type: "get_commands" }, { timeoutMs: 5_000 }).pipe(
        Effect.mapError(
          (cause) =>
            new ProviderAdapterRequestError({
              provider: PROVIDER,
              method: "get_commands",
              detail: "Prime approval extension handshake failed.",
              cause,
            }),
        ),
      );
      if (!isPrimeApprovalExtensionHandshake(response.data, extensionPath)) {
        return yield* new ProviderAdapterRequestError({
          provider: PROVIDER,
          method: "get_commands",
          detail: "Prime approval extension handshake did not identify the T3-owned extension.",
        });
      }
    });

    // Prime RLM children can outlive the turn that spawned them, so task
    // events use the most recent turn when no turn is active.
    const subagentTurnId = (ctx: PrimeSessionContext): TurnId | undefined =>
      ctx.activeTurnId ?? ctx.turns[ctx.turns.length - 1]?.id;

    const subagentLinkage = (state: PrimeSubagentState) => ({
      taskType: "subagent" as const,
      title: state.title,
      ...(state.model !== undefined ? { model: state.model } : {}),
    });

    const publishRlmChildUpdate = (
      ctx: PrimeSessionContext,
      event: typeof PrimeRlmChildUpdate.Type,
    ) =>
      Effect.gen(function* () {
        const child = event.child;
        const title =
          nonEmptyTrimmed(child.sessionName) ?? nonEmptyTrimmed(child.label) ?? child.id;
        const description = nonEmptyTrimmed(child.label) ?? title;
        const model = nonEmptyTrimmed(child.model);
        let state = ctx.subagents.get(child.id);
        const firstObservation = state === undefined;
        if (state === undefined) {
          state = {
            title,
            model,
            lastProgressKey: undefined,
            terminal: false,
          };
          ctx.subagents.set(child.id, state);
          const turnId = subagentTurnId(ctx);
          yield* publish({
            type: "task.started",
            ...(yield* stamp),
            provider: PROVIDER,
            providerInstanceId: boundInstanceId,
            threadId: ctx.threadId,
            ...(turnId !== undefined ? { turnId } : {}),
            payload: {
              taskId: RuntimeTaskId.make(child.id),
              ...subagentLinkage(state),
              description,
            },
          });
        } else {
          state.title = title;
          state.model = model ?? state.model;
        }

        const terminalStatus =
          child.status === "done"
            ? "completed"
            : child.status === "error"
              ? "failed"
              : child.status === "cancelled"
                ? "stopped"
                : undefined;
        const turnId = subagentTurnId(ctx);
        if (terminalStatus !== undefined) {
          if (state.terminal) {
            return;
          }
          state.terminal = true;
          const summary =
            terminalStatus === "completed"
              ? (nonEmptyTrimmed(child.answerPreview) ?? nonEmptyTrimmed(child.recap))
              : (nonEmptyTrimmed(child.error) ??
                nonEmptyTrimmed(child.answerPreview) ??
                nonEmptyTrimmed(child.recap));
          yield* publish({
            type: "task.completed",
            ...(yield* stamp),
            provider: PROVIDER,
            providerInstanceId: boundInstanceId,
            threadId: ctx.threadId,
            ...(turnId !== undefined ? { turnId } : {}),
            payload: {
              taskId: RuntimeTaskId.make(child.id),
              ...subagentLinkage(state),
              status: terminalStatus,
              ...(summary !== undefined ? { summary } : {}),
            },
          });
          return;
        }
        if (state.terminal) {
          return;
        }

        const summary = primeChildProgressSummary(child);
        const lastToolName = nonEmptyTrimmed(child.activity?.toolName);
        const progressKey = [
          child.status,
          title,
          model ?? "",
          child.activity?.kind ?? "",
          lastToolName ?? "",
          nonEmptyTrimmed(child.recap) ?? "",
        ].join("\0");
        // The admission snapshot already renders as task.started. Prime emits
        // rlm_child_update for every text delta, so only publish progress when
        // meaningful roster state changes.
        if (firstObservation && child.status === "queued" && child.activity === undefined) {
          state.lastProgressKey = progressKey;
          return;
        }
        if (state.lastProgressKey === progressKey) {
          return;
        }
        state.lastProgressKey = progressKey;
        yield* publish({
          type: "task.progress",
          ...(yield* stamp),
          provider: PROVIDER,
          providerInstanceId: boundInstanceId,
          threadId: ctx.threadId,
          ...(turnId !== undefined ? { turnId } : {}),
          payload: {
            taskId: RuntimeTaskId.make(child.id),
            ...subagentLinkage(state),
            description: summary,
            summary,
            status: child.status === "queued" ? "pending" : "running",
            ...(lastToolName !== undefined ? { lastToolName } : {}),
          },
        });
      });

    const handlePrimeEvent = (ctx: PrimeSessionContext, raw: unknown) =>
      Effect.gen(function* () {
        if (sessions.get(ctx.threadId) !== ctx || ctx.stopped) {
          return;
        }
        // RLM children can settle after the spawning turn ends, so handle
        // their native snapshot event before the active-turn guard.
        const rlmChildUpdate = decodeRlmChildUpdate(raw);
        if (Exit.isSuccess(rlmChildUpdate)) {
          yield* publishRlmChildUpdate(ctx, rlmChildUpdate.value);
          return;
        }
        const turnId = ctx.activeTurnId;
        if (turnId === undefined) {
          return;
        }

        const userInput = decodeExtensionUserInputRequest(raw);
        if (Exit.isSuccess(userInput)) {
          const requestId = userInput.value.id.trim();
          if (
            !requestId ||
            ctx.pendingUserInputs.has(requestId) ||
            ctx.pendingApprovals.has(requestId)
          ) {
            return;
          }
          const title = userInput.value.title.trim() || "Prime Agent input";
          const options =
            userInput.value.method === "select"
              ? userInput.value.options.map((option) => option.trim()).filter(Boolean)
              : [];
          const question =
            userInput.value.method === "input" && userInput.value.placeholder?.trim()
              ? `${title}\n\nPlaceholder: ${userInput.value.placeholder.trim()}`
              : userInput.value.method === "editor" && userInput.value.prefill?.trim()
                ? `${title}\n\nCurrent text:\n${userInput.value.prefill.trim()}`
                : title;
          ctx.pendingUserInputs.set(requestId, {
            turnId,
            request: userInput.value,
            options,
          });
          yield* publish({
            type: "user-input.requested",
            ...(yield* stamp),
            provider: PROVIDER,
            providerInstanceId: boundInstanceId,
            threadId: ctx.threadId,
            turnId,
            requestId: RuntimeRequestId.make(requestId),
            providerRefs: { providerRequestId: requestId },
            payload: {
              questions: [
                {
                  id: requestId,
                  header: title,
                  question,
                  options: options.map((option) => ({
                    label: option,
                    description: option,
                  })),
                  multiSelect: false,
                },
              ],
            },
          });
          return;
        }

        const confirm = decodeExtensionConfirmRequest(raw);
        if (Exit.isSuccess(confirm)) {
          const requestId = confirm.value.id.trim();
          if (!requestId || ctx.pendingApprovals.has(requestId)) {
            return;
          }
          const detail = `${confirm.value.title.trim()}\n\n${confirm.value.message.trim()}`.trim();
          const toolName = confirm.value.message.split("\n", 1)[0]?.trim().toLowerCase();
          const requestType =
            toolName === "ipython" || toolName === "bash"
              ? "command_execution_approval"
              : "dynamic_tool_call";
          ctx.pendingApprovals.set(requestId, {
            requestType,
            turnId,
            detail,
            request: confirm.value,
          });
          yield* publish({
            type: "request.opened",
            ...(yield* stamp),
            provider: PROVIDER,
            providerInstanceId: boundInstanceId,
            threadId: ctx.threadId,
            turnId,
            requestId: RuntimeRequestId.make(requestId),
            providerRefs: { providerRequestId: requestId },
            payload: {
              requestType,
              ...(detail ? { detail } : {}),
              args: confirm.value,
            },
          });
          return;
        }

        const started = decodeAgentStart(raw);
        if (Exit.isSuccess(started)) {
          ctx.session = {
            ...ctx.session,
            status: "running",
            activeTurnId: turnId,
            updatedAt: yield* nowIso,
          };
          return;
        }

        const delta = decodeTextDelta(raw);
        if (Exit.isSuccess(delta)) {
          const text = delta.value.assistantMessageEvent.delta;
          if (text.length === 0) {
            return;
          }
          const turn = ctx.turns.find((entry) => entry.id === turnId);
          if (turn) {
            turn.items.push({ type: "assistant_text", text });
          }
          yield* publish({
            type: "content.delta",
            ...(yield* stamp),
            provider: PROVIDER,
            providerInstanceId: boundInstanceId,
            threadId: ctx.threadId,
            turnId,
            payload: {
              streamKind: "assistant_text",
              delta: text,
              ...(delta.value.assistantMessageEvent.contentIndex !== undefined
                ? { contentIndex: delta.value.assistantMessageEvent.contentIndex }
                : {}),
            },
          });
          return;
        }

        const thinking = decodeThinkingDelta(raw);
        if (Exit.isSuccess(thinking)) {
          const text = thinking.value.assistantMessageEvent.delta;
          if (text.length === 0) {
            return;
          }
          yield* publish({
            type: "content.delta",
            ...(yield* stamp),
            provider: PROVIDER,
            providerInstanceId: boundInstanceId,
            threadId: ctx.threadId,
            turnId,
            payload: {
              streamKind: "reasoning_text",
              delta: text,
              ...(thinking.value.assistantMessageEvent.contentIndex !== undefined
                ? { contentIndex: thinking.value.assistantMessageEvent.contentIndex }
                : {}),
            },
          });
          return;
        }

        const toolStarted = decodeToolExecutionStart(raw);
        if (Exit.isSuccess(toolStarted)) {
          const detail = toolResultDetail(toolStarted.value.args);
          yield* publish({
            type: "item.started",
            ...(yield* stamp),
            provider: PROVIDER,
            providerInstanceId: boundInstanceId,
            threadId: ctx.threadId,
            turnId,
            itemId: RuntimeItemId.make(toolStarted.value.toolCallId),
            payload: {
              itemType: toToolLifecycleItemType(toolStarted.value.toolName),
              status: "inProgress",
              title: toolStarted.value.toolName,
              ...(detail ? { detail } : {}),
              data: { args: toolStarted.value.args },
            },
          });
          return;
        }

        const toolUpdated = decodeToolExecutionUpdate(raw);
        if (Exit.isSuccess(toolUpdated)) {
          // Prime's partialResult is cumulative. Replace the previous result;
          // never append, or the T3 websocket balloons.
          const detail = toolResultDetail(toolUpdated.value.partialResult);
          yield* publish({
            type: "item.updated",
            ...(yield* stamp),
            provider: PROVIDER,
            providerInstanceId: boundInstanceId,
            threadId: ctx.threadId,
            turnId,
            itemId: RuntimeItemId.make(toolUpdated.value.toolCallId),
            payload: {
              itemType: toToolLifecycleItemType(toolUpdated.value.toolName),
              status: "inProgress",
              title: toolUpdated.value.toolName,
              ...(detail ? { detail } : {}),
              data: {
                args: toolUpdated.value.args,
                result: toolUpdated.value.partialResult,
              },
            },
          });
          return;
        }

        const toolEnded = decodeToolExecutionEnd(raw);
        if (Exit.isSuccess(toolEnded)) {
          const detail = toolResultDetail(toolEnded.value.result);
          yield* publish({
            type: "item.completed",
            ...(yield* stamp),
            provider: PROVIDER,
            providerInstanceId: boundInstanceId,
            threadId: ctx.threadId,
            turnId,
            itemId: RuntimeItemId.make(toolEnded.value.toolCallId),
            payload: {
              itemType: toToolLifecycleItemType(toolEnded.value.toolName),
              status: toolEnded.value.isError ? "failed" : "completed",
              title: toolEnded.value.toolName,
              ...(detail ? { detail } : {}),
              data: { result: toolEnded.value.result },
            },
          });
          return;
        }

        const ended = decodeAgentEnd(raw);
        if (Exit.isSuccess(ended)) {
          yield* cancelPendingRequests(ctx).pipe(
            Effect.catch(() =>
              cancelPendingRequests(ctx, { sendResponse: false }).pipe(Effect.ignore),
            ),
          );
          const messages = ended.value.messages ?? [];
          const turn = ctx.turns.find((entry) => entry.id === turnId);
          if (turn) {
            turn.items.push(...messages);
          }
          ctx.latestAgentEndMessages = messages;
          const completedAt = yield* nowIso;
          const { activeTurnId: _activeTurnId, ...readySession } = ctx.session;
          ctx.activeTurnId = undefined;
          ctx.settledTurnId = turnId;
          if (isAbortedTurn(messages)) {
            ctx.session = { ...readySession, status: "ready", updatedAt: completedAt };
            yield* publishTurnInterrupted({
              threadId: ctx.threadId,
              turnId,
              reason: "Interrupted by user.",
            });
            return;
          }
          const failure = primeFailureFromMessages(messages);
          const usage = usageFromMessages(messages);
          ctx.session = {
            ...readySession,
            status: "ready",
            updatedAt: completedAt,
            ...(failure ? { lastError: failure.errorMessage } : {}),
          };
          yield* publish({
            type: "turn.completed",
            ...(yield* stamp),
            provider: PROVIDER,
            providerInstanceId: boundInstanceId,
            threadId: ctx.threadId,
            turnId,
            payload: {
              state: failure ? "failed" : "completed",
              stopReason: failure?.stopReason ?? "stop",
              ...(failure ? { errorMessage: failure.errorMessage } : {}),
              ...(usage !== undefined ? { usage } : {}),
            },
          });
        }
      });

    const handleUnexpectedExit = (ctx: PrimeSessionContext) =>
      Effect.gen(function* () {
        if (ctx.stopped || sessions.get(ctx.threadId) !== ctx) {
          return;
        }
        yield* cancelPendingRequests(ctx, { sendResponse: false }).pipe(Effect.ignore);
        ctx.stopped = true;
        sessions.delete(ctx.threadId);
        const turnId = ctx.activeTurnId;
        ctx.activeTurnId = undefined;
        const message = "Prime RPC process exited.";
        const updatedAt = yield* nowIso;
        const { activeTurnId: _activeTurnId, ...failedSession } = ctx.session;
        ctx.session = { ...failedSession, status: "error", updatedAt, lastError: message };
        if (turnId !== undefined) {
          yield* publishTurnInterrupted({
            threadId: ctx.threadId,
            turnId,
            reason: message,
          });
        }
        yield* publish({
          type: "session.exited",
          ...(yield* stamp),
          provider: PROVIDER,
          providerInstanceId: boundInstanceId,
          threadId: ctx.threadId,
          payload: { reason: message, recoverable: false, exitKind: "error" },
        });
        yield* Scope.close(ctx.scope, Exit.void).pipe(Effect.forkDetach, Effect.ignore);
      });

    const abortIfBusy = (ctx: PrimeSessionContext) =>
      ctx.activeTurnId === undefined
        ? Effect.void
        : ctx.rpc.request({ type: "abort" }).pipe(Effect.ignore);

    const stopSessionInternal = (ctx: PrimeSessionContext) =>
      Effect.gen(function* () {
        if (ctx.stopped) {
          return;
        }
        yield* cancelPendingRequests(ctx);
        ctx.stopped = true;
        sessions.delete(ctx.threadId);
        const turnId = ctx.activeTurnId;
        yield* abortIfBusy(ctx);
        ctx.activeTurnId = undefined;
        if (turnId !== undefined) {
          yield* publishTurnInterrupted({
            threadId: ctx.threadId,
            turnId,
            reason: "Session stopped.",
          });
        }
        yield* ctx.rpc.close;
        yield* Scope.close(ctx.scope, Exit.void).pipe(Effect.ignore);
        yield* publish({
          type: "session.exited",
          ...(yield* stamp),
          provider: PROVIDER,
          providerInstanceId: boundInstanceId,
          threadId: ctx.threadId,
          payload: { exitKind: "graceful" },
        });
      });

    const startSession: PrimeAdapterShape["startSession"] = (input) =>
      Effect.gen(function* () {
        if (input.provider !== undefined && input.provider !== PROVIDER) {
          return yield* new ProviderAdapterValidationError({
            provider: PROVIDER,
            operation: "startSession",
            issue: `Expected driver '${PROVIDER}' but received '${input.provider}'.`,
          });
        }
        const cwd = (input.cwd?.trim() || serverConfig.cwd).trim();
        if (!cwd) {
          return yield* new ProviderAdapterValidationError({
            provider: PROVIDER,
            operation: "startSession",
            issue: "cwd is required and must be non-empty.",
          });
        }
        const supervised = input.runtimeMode === "approval-required";
        if (!supervised && input.runtimeMode !== "full-access") {
          return yield* new ProviderAdapterValidationError({
            provider: PROVIDER,
            operation: "startSession",
            issue: `Prime Agent does not support runtime mode '${input.runtimeMode}'.`,
          });
        }
        const launchArgs = tokenizeCliArgs(primeSettings.launchArgs);
        if (supervised) {
          const unsafeArg = launchArgs.find(isUnsafeSupervisedLaunchArg);
          if (unsafeArg !== undefined) {
            return yield* new ProviderAdapterValidationError({
              provider: PROVIDER,
              operation: "startSession",
              issue: `Launch argument '${unsafeArg}' can override the T3 approval extension.`,
            });
          }
        }
        const existing = sessions.get(input.threadId);
        if (existing !== undefined) {
          yield* stopSessionInternal(existing);
        }

        const threadSessionDir = path.join(
          serverConfig.baseDir,
          "prime-agent",
          "sessions",
          String(input.threadId),
        );
        const daemonSocket = path.join(serverConfig.baseDir, "prime-agent", "daemon.sock");
        yield* fileSystem.makeDirectory(threadSessionDir, { recursive: true }).pipe(
          Effect.andThen(fileSystem.makeDirectory(path.dirname(daemonSocket), { recursive: true })),
          Effect.mapError(
            (cause) =>
              new ProviderAdapterRequestError({
                provider: PROVIDER,
                method: "prepare",
                detail: "Could not prepare the Prime Agent session directory.",
                cause,
              }),
          ),
        );
        const catalogExtensionPath = yield* preparePrimeOpenRouterCatalogExtension(
          serverConfig.baseDir,
        ).pipe(
          Effect.provideService(FileSystem.FileSystem, fileSystem),
          Effect.provideService(Path.Path, path),
          Effect.mapError(
            (cause) =>
              new ProviderAdapterRequestError({
                provider: PROVIDER,
                method: "prepare",
                detail: "Could not install the OpenRouter catalog extension.",
                cause,
              }),
          ),
        );
        const approvalExtensionPath = supervised
          ? yield* preparePrimeApprovalExtension(serverConfig.baseDir).pipe(
              Effect.provideService(FileSystem.FileSystem, fileSystem),
              Effect.provideService(Path.Path, path),
              Effect.mapError(
                (cause) =>
                  new ProviderAdapterRequestError({
                    provider: PROVIDER,
                    method: "prepare",
                    detail: "Could not install the T3 approval extension.",
                    cause,
                  }),
              ),
            )
          : undefined;

        const resume = parsePrimeResume(input.resumeCursor);
        if (resume !== undefined) {
          yield* requirePersistedResumeFile({
            threadSessionDir,
            sessionFile: resume.sessionFile,
          });
        }

        const sessionScope = yield* Scope.make("sequential");
        let sessionScopeTransferred = false;
        yield* Effect.addFinalizer(() =>
          sessionScopeTransferred ? Effect.void : Scope.close(sessionScope, Exit.void),
        );

        const args = [
          "--mode",
          "rpc",
          "--cwd",
          cwd,
          "--session-dir",
          threadSessionDir,
          "--daemon-socket",
          daemonSocket,
          ...(resume !== undefined ? ["--resume", resume.sessionFile] : []),
          ...launchArgs,
          ...(approvalExtensionPath !== undefined ? ["--no-extensions"] : []),
          "--extension",
          catalogExtensionPath,
          ...(approvalExtensionPath !== undefined
            ? [
                "--extension",
                approvalExtensionPath,
                `--${PRIME_APPROVAL_EXTENSION_MODE_FLAG}=approval-required`,
              ]
            : []),
        ];
        const rpc = yield* spawnPrimeRpcClient({
          command: primeSettings.binaryPath || "prime-agent",
          args,
          cwd,
          ...(options?.environment ? { environment: options.environment } : {}),
        }).pipe(
          Effect.provideService(Scope.Scope, sessionScope),
          Effect.provideService(ChildProcessSpawner.ChildProcessSpawner, childProcessSpawner),
          Effect.mapError((cause) => toRpcRequestError("spawn", cause)),
        );

        if (approvalExtensionPath !== undefined) {
          yield* requireApprovalExtensionHandshake(rpc, approvalExtensionPath);
        }

        const stateResponse = yield* rpc
          .request({ type: "get_state" })
          .pipe(Effect.mapError((cause) => toRpcRequestError("get_state", cause)));
        const state = yield* decodePrimeState(stateResponse.data).pipe(
          Effect.mapError(
            (cause) =>
              new ProviderAdapterRequestError({
                provider: PROVIDER,
                method: "get_state",
                detail: "Prime returned an invalid state snapshot.",
                cause,
              }),
          ),
        );
        yield* requireSessionFileInsideThreadDir({
          threadSessionDir,
          sessionFile: state.sessionFile,
          detail: "Prime returned a session file outside this thread's session directory.",
        });
        if (resume !== undefined && state.sessionId !== resume.sessionId) {
          return yield* new ProviderAdapterRequestError({
            provider: PROVIDER,
            method: "resume",
            detail: "Prime session id did not match the persisted resume cursor.",
          });
        }

        const selected = yield* applyModelSelection({
          rpc,
          modelSelection:
            input.modelSelection?.instanceId === boundInstanceId ? input.modelSelection : undefined,
          currentModelSlug: modelSlugFromPrimeState(state),
          currentThinkingLevel: state.thinkingLevel,
        });
        const resumeCursor = {
          schemaVersion: PRIME_RESUME_VERSION,
          sessionId: state.sessionId,
          sessionFile: state.sessionFile,
        };
        const createdAt = yield* nowIso;
        const session: ProviderSession = {
          provider: PROVIDER,
          providerInstanceId: boundInstanceId,
          status: "ready",
          runtimeMode: input.runtimeMode,
          cwd,
          ...(selected.slug ? { model: selected.slug } : {}),
          threadId: input.threadId,
          resumeCursor,
          createdAt,
          updatedAt: createdAt,
        };
        const ctx: PrimeSessionContext = {
          threadId: input.threadId,
          rpc,
          scope: sessionScope,
          session,
          currentModelSlug: selected.slug,
          currentThinkingLevel: selected.thinking,
          activeTurnId: undefined,
          turns: [],
          latestAgentEndMessages: undefined,
          pendingApprovals: new Map(),
          pendingUserInputs: new Map(),
          turnStarted: false,
          stopped: false,
          settledTurnId: undefined,
          subagents: new Map(),
        };
        sessions.set(input.threadId, ctx);
        sessionScopeTransferred = true;
        // Take RPC events off the stdout fiber immediately. If the subscriber
        // waits on DateTime/crypto while stdout is blocked, abort responses
        // never get read.
        const incoming = yield* Queue.unbounded<unknown>();
        yield* rpc.events.pipe(
          Stream.runForEach((event) => Queue.offer(incoming, event)),
          Effect.ensuring(Queue.shutdown(incoming)),
          Effect.forkIn(sessionScope),
        );
        yield* Stream.fromQueue(incoming).pipe(
          Stream.runForEach((event) => handlePrimeEvent(ctx, event)),
          Effect.ensuring(handleUnexpectedExit(ctx)),
          Effect.forkIn(sessionScope),
        );
        yield* publish({
          type: "session.started",
          ...(yield* stamp),
          provider: PROVIDER,
          providerInstanceId: boundInstanceId,
          threadId: input.threadId,
          payload: { message: "Prime Agent session started" },
        });
        yield* publish({
          type: "thread.started",
          ...(yield* stamp),
          provider: PROVIDER,
          providerInstanceId: boundInstanceId,
          threadId: input.threadId,
          payload: { providerThreadId: state.sessionId },
        });
        return session;
      }).pipe(Effect.scoped);

    const buildTurnAttachments = (input: ProviderSendTurnInput) =>
      Effect.forEach(input.attachments ?? [], (attachment) => {
        const attachmentPath = resolveAttachmentPath({
          attachmentsDir: serverConfig.attachmentsDir,
          attachment,
        });
        if (!attachmentPath) {
          return Effect.fail(
            new ProviderAdapterRequestError({
              provider: PROVIDER,
              method: "prompt",
              detail: `Invalid attachment id '${attachment.id}'.`,
            }),
          );
        }
        return fileSystem.readFile(attachmentPath).pipe(
          Effect.map((bytes) => ({
            type: "image" as const,
            data: Buffer.from(bytes).toString("base64"),
            mimeType: attachment.mimeType,
          })),
          Effect.mapError(
            (cause) =>
              new ProviderAdapterRequestError({
                provider: PROVIDER,
                method: "prompt",
                detail: cause.message,
                cause,
              }),
          ),
        );
      });

    const steerRunningTurn = (
      ctx: PrimeSessionContext,
      input: ProviderSendTurnInput,
      turnId: TurnId,
    ) =>
      Effect.gen(function* () {
        const message = input.input?.trim() ?? "";
        const images = yield* buildTurnAttachments(input);
        if (!message && images.length === 0) {
          return yield* new ProviderAdapterValidationError({
            provider: PROVIDER,
            operation: "sendTurn",
            issue: "Turn requires non-empty text or at least one image.",
          });
        }
        yield* ctx.rpc
          .request({
            type: "steer",
            message,
            ...(images.length > 0 ? { images } : {}),
          })
          .pipe(Effect.mapError((cause) => toRpcRequestError("steer", cause)));
        const turn = ctx.turns.find((entry) => entry.id === turnId);
        turn?.items.push(
          ...(message ? [{ type: "user_text", text: message }] : []),
          ...images.map((image) => ({ type: "user_image", mimeType: image.mimeType })),
        );
        ctx.session = { ...ctx.session, updatedAt: yield* nowIso };
        return {
          threadId: input.threadId,
          turnId,
          ...(ctx.session.resumeCursor !== undefined
            ? { resumeCursor: ctx.session.resumeCursor }
            : {}),
        };
      });

    const sendTurn: PrimeAdapterShape["sendTurn"] = (input) =>
      Effect.gen(function* () {
        const ctx = yield* requireSession(input.threadId);
        if (ctx.activeTurnId !== undefined) {
          // Prime queues mid-turn input as steering on the live turn.
          return yield* steerRunningTurn(ctx, input, ctx.activeTurnId);
        }
        const selected = yield* applyModelSelection({
          rpc: ctx.rpc,
          modelSelection:
            input.modelSelection?.instanceId === boundInstanceId ? input.modelSelection : undefined,
          currentModelSlug: ctx.currentModelSlug,
          currentThinkingLevel: ctx.currentThinkingLevel,
        });
        ctx.currentModelSlug = selected.slug;
        ctx.currentThinkingLevel = selected.thinking;
        if (selected.slug !== undefined) {
          ctx.session = { ...ctx.session, model: selected.slug };
        }
        const message = input.input?.trim() ?? "";
        const images = yield* buildTurnAttachments(input);
        if (!message && images.length === 0) {
          return yield* new ProviderAdapterValidationError({
            provider: PROVIDER,
            operation: "sendTurn",
            issue: "Turn requires non-empty text or at least one image.",
          });
        }

        const turnId = TurnId.make(yield* randomUUIDv4);
        ctx.activeTurnId = turnId;
        ctx.latestAgentEndMessages = undefined;
        ctx.turnStarted = true;
        ctx.settledTurnId = undefined;
        ctx.turns.push({
          id: turnId,
          items: [
            ...(message ? [{ type: "user_text", text: message }] : []),
            ...images.map((image) => ({ type: "user_image", mimeType: image.mimeType })),
          ],
        });
        ctx.session = {
          ...ctx.session,
          status: "running",
          activeTurnId: turnId,
          updatedAt: yield* nowIso,
        };
        yield* publish({
          type: "turn.started",
          ...(yield* stamp),
          provider: PROVIDER,
          providerInstanceId: boundInstanceId,
          threadId: input.threadId,
          turnId,
          payload: ctx.session.model ? { model: ctx.session.model } : {},
        });

        yield* ctx.rpc
          .request({
            type: "prompt",
            message,
            streamingBehavior: "followUp",
            ...(images.length > 0 ? { images } : {}),
          })
          .pipe(
            Effect.mapError((cause) => toRpcRequestError("prompt", cause)),
            Effect.tapError((error) =>
              Effect.gen(function* () {
                if (ctx.activeTurnId !== turnId) {
                  return;
                }
                const updatedAt = yield* nowIso;
                const { activeTurnId: _activeTurnId, ...readySession } = ctx.session;
                ctx.activeTurnId = undefined;
                ctx.settledTurnId = turnId;
                ctx.session = {
                  ...readySession,
                  status: "ready",
                  updatedAt,
                  lastError: error.detail,
                };
                yield* publish({
                  type: "turn.aborted",
                  ...(yield* stamp),
                  provider: PROVIDER,
                  providerInstanceId: boundInstanceId,
                  threadId: input.threadId,
                  turnId,
                  payload: { reason: error.detail },
                });
              }),
            ),
          );

        return {
          threadId: input.threadId,
          turnId,
          ...(ctx.session.resumeCursor !== undefined
            ? { resumeCursor: ctx.session.resumeCursor }
            : {}),
        };
      });

    const interruptTurn: PrimeAdapterShape["interruptTurn"] = (threadId, turnId) =>
      Effect.gen(function* () {
        const ctx = yield* requireSession(threadId);
        const activeTurnId = ctx.activeTurnId;
        if (activeTurnId === undefined || (turnId !== undefined && activeTurnId !== turnId)) {
          return;
        }
        yield* cancelPendingRequests(ctx);
        yield* ctx.rpc
          .request({ type: "abort" })
          .pipe(Effect.mapError((cause) => toRpcRequestError("abort", cause)));
        if (ctx.settledTurnId === activeTurnId) {
          return;
        }
        const updatedAt = yield* nowIso;
        const { activeTurnId: _activeTurnId, ...readySession } = ctx.session;
        if (ctx.activeTurnId === activeTurnId) {
          ctx.activeTurnId = undefined;
        }
        ctx.settledTurnId = activeTurnId;
        ctx.session = { ...readySession, status: "ready", updatedAt };
        yield* publishTurnInterrupted({
          threadId,
          turnId: activeTurnId,
          reason: "Interrupted by user.",
        });
      });

    const respondToRequest: PrimeAdapterShape["respondToRequest"] = (
      threadId,
      approvalRequestId,
      decision,
    ) =>
      Effect.gen(function* () {
        const ctx = yield* requireSession(threadId);
        const requestId = String(approvalRequestId);
        if (decision === "acceptForSession") {
          return yield* new ProviderAdapterRequestError({
            provider: PROVIDER,
            method: "respondToRequest",
            detail: "Prime Agent does not support persistent approval decisions yet.",
          });
        }
        yield* resolvePrimeApproval(ctx, requestId, decision);
      });

    const respondToUserInput: PrimeAdapterShape["respondToUserInput"] = (
      threadId,
      approvalRequestId,
      answers,
    ) =>
      Effect.gen(function* () {
        const ctx = yield* requireSession(threadId);
        const requestId = String(approvalRequestId);
        const pending = ctx.pendingUserInputs.get(requestId);
        if (pending === undefined) {
          return yield* new ProviderAdapterRequestError({
            provider: PROVIDER,
            method: "respondToUserInput",
            detail: `Prime user-input request '${requestId}' is not pending.`,
          });
        }
        const answer = answers[requestId];
        if (typeof answer !== "string") {
          return yield* new ProviderAdapterValidationError({
            provider: PROVIDER,
            operation: "respondToUserInput",
            issue: `Prime user-input request '${requestId}' requires a text answer.`,
          });
        }
        if (pending.request.method === "select" && !pending.options.includes(answer)) {
          return yield* new ProviderAdapterValidationError({
            provider: PROVIDER,
            operation: "respondToUserInput",
            issue: `Prime selection '${requestId}' requires one of its offered options.`,
          });
        }
        yield* resolvePrimeUserInput(ctx, requestId, answer);
      });

    yield* Effect.addFinalizer(() =>
      Effect.forEach(Array.from(sessions.values()), stopSessionInternal, { discard: true }).pipe(
        Effect.tap(() => PubSub.shutdown(runtimeEvents)),
        Effect.ignore,
      ),
    );

    return {
      provider: PROVIDER,
      capabilities: { sessionModelSwitch: "in-session" },
      startSession,
      sendTurn,
      interruptTurn,
      respondToRequest,
      respondToUserInput,
      stopSession: (threadId) => requireSession(threadId).pipe(Effect.flatMap(stopSessionInternal)),
      listSessions: () => Effect.sync(() => Array.from(sessions.values(), (ctx) => ctx.session)),
      hasSession: (threadId) => Effect.sync(() => sessions.has(threadId)),
      readThread: (threadId) =>
        requireSession(threadId).pipe(Effect.map((ctx) => ({ threadId, turns: ctx.turns }))),
      rollbackThread: (threadId) =>
        requireSession(threadId).pipe(
          Effect.flatMap(() =>
            Effect.fail(
              new ProviderAdapterValidationError({
                provider: PROVIDER,
                operation: "rollbackThread",
                issue: "Prime Agent does not expose durable turn rollback yet.",
              }),
            ),
          ),
        ),
      stopAll: () =>
        Effect.forEach(Array.from(sessions.values()), stopSessionInternal, { discard: true }),
      streamEvents: Stream.fromPubSub(runtimeEvents),
    } satisfies PrimeAdapterShape;
  });
}
