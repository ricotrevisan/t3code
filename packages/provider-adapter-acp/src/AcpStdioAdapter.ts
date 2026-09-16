import type {
  ProviderAdapterPackageV1,
  ProviderAdapterProcessV1,
  ProviderAdapterV1Error as ProviderAdapterError,
} from "@t3tools/provider-adapter";
import { ProviderAdapterV1Error, defineProviderAdapterV1 } from "@t3tools/provider-adapter";
import {
  ApprovalRequestId,
  EventId,
  RuntimeRequestId,
  ProviderItemId,
  ThreadId,
  TurnId,
  type ProviderAdapterManifestV1,
  type ProviderAdapterPackageReference,
  type ProviderAdapterProtocolCapabilitiesV1,
  type ProviderApprovalDecision,
  type ProviderRuntimeEvent,
  type ProviderSession,
  type ProviderSessionStartInput,
  type ProviderSendTurnInput,
  type ServerProvider,
} from "@t3tools/contracts";
import * as DateTime from "effect/DateTime";
import * as Deferred from "effect/Deferred";
import * as Effect from "effect/Effect";
import * as PubSub from "effect/PubSub";
import * as Schema from "effect/Schema";
import * as Scope from "effect/Scope";
import * as Semaphore from "effect/Semaphore";
import * as Stream from "effect/Stream";
import * as AcpSchema from "effect-acp/schema";

import {
  findAcpReasoningSelection,
  getAcpConfigOptionUpdate,
  projectAcpConfigOptions,
} from "./AcpConfigOptions.ts";
import {
  finishAcpTurn,
  makeAcpEventMapperState,
  mapAcpSessionUpdate,
  type AcpEventMapperState,
} from "./AcpEventMapper.ts";
import * as AcpHostConnection from "./AcpHostConnection.ts";

export const AcpResumeCursorV1 = Schema.Struct({
  kind: Schema.Literal("acp-v1"),
  protocolVersion: Schema.Literal(1),
  sessionId: Schema.String.check(Schema.isMinLength(1)),
});
export type AcpResumeCursorV1 = typeof AcpResumeCursorV1.Type;

export const AcpStdioAdapterConfig = Schema.Struct({
  command: Schema.String.check(Schema.isMinLength(1)),
  args: Schema.optionalKey(Schema.Array(Schema.String)),
  cwd: Schema.optionalKey(Schema.String.check(Schema.isMinLength(1))),
  env: Schema.optionalKey(Schema.Record(Schema.String, Schema.String)),
  mcpServers: Schema.optionalKey(Schema.Array(AcpSchema.McpServer)),
});
export type AcpStdioAdapterConfig = typeof AcpStdioAdapterConfig.Type;

export interface AcpStdioAdapterDefinitionV1 {
  readonly manifest: ProviderAdapterManifestV1;
  readonly defaultConfig: () => AcpStdioAdapterConfig;
  readonly clientInfo?: AcpSchema.Implementation | undefined;
}

type AcpClient = AcpHostConnection.AcpHostConnection["client"];
type PermissionResponse = AcpSchema.RequestPermissionResponse;

interface PendingPermission {
  readonly id: string;
  readonly request: AcpSchema.RequestPermissionRequest;
  readonly response: Deferred.Deferred<PermissionResponse>;
}

interface ActiveTurn {
  readonly id: TurnId;
  readonly items: Array<unknown>;
  cancelled: boolean;
  terminalClaimed: boolean;
  readonly terminalDone: Deferred.Deferred<void>;
}

interface SessionState {
  session: ProviderSession;
  readonly acpSessionId: string;
  readonly owner: LiveProcess;
  readonly commandLock: Semaphore.Semaphore;
  readonly mapperLock: Semaphore.Semaphore;
  mapper: AcpEventMapperState;
  readonly turns: Array<{ readonly id: TurnId; readonly items: ReadonlyArray<unknown> }>;
  activeTurn: ActiveTurn | undefined;
  readonly pendingPermissions: Map<string, PendingPermission>;
  snapshotSequence: number;
  ready: boolean;
  closed: boolean;
}

interface LiveProcess {
  readonly process: ProviderAdapterProcessV1;
  readonly client: AcpClient;
  readonly capabilities: NonNullable<AcpSchema.InitializeResponse["agentCapabilities"]> | undefined;
  readonly attachedThreads: Set<ThreadId>;
  exited: boolean;
  closing: boolean;
}

const supportedFeatureCeiling = new Set<ProviderAdapterProtocolCapabilitiesV1["features"][number]>([
  "session.resume",
  "turn.interrupt",
  "request.approval",
  "model.discovery",
  "model.switch",
  "reasoning.selection",
  "stream.reasoning",
  "stream.tool-lifecycle",
  "stream.context",
]);

const MAX_TRANSCRIPT_TURNS = 100;
const MAX_TRANSCRIPT_ITEMS_PER_TURN = 200;
const MAX_TRANSCRIPT_STRING_CHARS = 16_000;
const MAX_TRANSCRIPT_COLLECTION_ENTRIES = 32;
const MAX_TRANSCRIPT_DEPTH = 4;

const sanitizeTranscriptValue = (value: unknown, depth = 0): unknown => {
  if (typeof value === "string") return value.slice(0, MAX_TRANSCRIPT_STRING_CHARS);
  if (value === null || typeof value !== "object") return value;
  if (depth >= MAX_TRANSCRIPT_DEPTH) return "[truncated]";
  if (Array.isArray(value)) {
    return value
      .slice(0, MAX_TRANSCRIPT_COLLECTION_ENTRIES)
      .map((entry) => sanitizeTranscriptValue(entry, depth + 1));
  }
  return Object.fromEntries(
    Object.entries(value)
      .slice(0, MAX_TRANSCRIPT_COLLECTION_ENTRIES)
      .map(([key, entry]) => [key, sanitizeTranscriptValue(entry, depth + 1)]),
  );
};

const packageReference = (
  manifest: ProviderAdapterManifestV1,
): ProviderAdapterPackageReference => ({
  id: manifest.id,
  version: manifest.version,
  protocolVersion: manifest.protocolVersion,
});

const adapterError = (operation: string, detail: string, cause?: unknown) =>
  new ProviderAdapterV1Error({ operation, detail, ...(cause === undefined ? {} : { cause }) });

const detailOf = (cause: unknown): string =>
  cause instanceof Error ? cause.message : typeof cause === "string" ? cause : String(cause);

const fail = (operation: string, detail: string): Effect.Effect<never, ProviderAdapterError> =>
  Effect.fail(adapterError(operation, detail));

const protect = <A, E>(
  operation: string,
  effect: Effect.Effect<A, E>,
): Effect.Effect<A, ProviderAdapterError> =>
  effect.pipe(
    Effect.mapError((cause) => adapterError(operation, detailOf(cause), cause)),
    Effect.catchDefect((cause) => Effect.fail(adapterError(operation, detailOf(cause), cause))),
  );

const decodeResumeCursor = Schema.decodeUnknownEffect(AcpResumeCursorV1);
const isProviderAdapterError = Schema.is(ProviderAdapterV1Error);

const now = (): string => DateTime.formatIso(DateTime.nowUnsafe());

const supportsResume = (live: LiveProcess): boolean =>
  live.capabilities?.sessionCapabilities?.resume != null;

const supportsClose = (live: LiveProcess): boolean =>
  live.capabilities?.sessionCapabilities?.close != null;

const permissionRequestType = (
  kind: AcpSchema.ToolKind | null | undefined,
): "command_execution_approval" | "file_change_approval" | "unknown" => {
  switch (kind) {
    case "execute":
      return "command_execution_approval";
    case "edit":
    case "delete":
    case "move":
      return "file_change_approval";
    default:
      return "unknown";
  }
};

const permissionKindForDecision = (
  decision: ProviderApprovalDecision,
): AcpSchema.PermissionOption["kind"] | undefined => {
  switch (decision) {
    case "accept":
      return "allow_once";
    case "acceptForSession":
      return "allow_always";
    case "decline":
      return "reject_once";
    case "cancel":
      return undefined;
  }
};

/** Define one trusted, host-supervised multiplexed ACP stdio adapter package. */
export const defineAcpStdioAdapterV1 = (
  definition: AcpStdioAdapterDefinitionV1,
): ProviderAdapterPackageV1<AcpStdioAdapterConfig> =>
  defineProviderAdapterV1({
    manifest: definition.manifest,
    configSchema: AcpStdioAdapterConfig,
    defaultConfig: definition.defaultConfig,
    create: (input, host) =>
      Effect.gen(function* () {
        if (
          definition.manifest.transport.kind !== "supervised-stdio" ||
          definition.manifest.transport.protocol !== "acp-v1" ||
          definition.manifest.transport.sessionConcurrency !== "multiplexed"
        ) {
          return yield* adapterError(
            "create",
            "ACP stdio adapters require acp-v1 multiplexed transport metadata.",
          );
        }
        const ownerScope = yield* Scope.Scope;
        const lifecycleLock = yield* Semaphore.make(1);
        const events = yield* PubSub.unbounded<ProviderRuntimeEvent>();
        const snapshotChanges = yield* PubSub.unbounded<ServerProvider>();
        const sessions = new Map<ThreadId, SessionState>();
        const sessionsByAcpId = new Map<string, SessionState>();
        let liveProcess: LiveProcess | undefined;
        let sequence = 0;
        let snapshotSequence = 0;

        const negotiatedCapabilities: ProviderAdapterProtocolCapabilitiesV1 = {
          protocolVersion: 1,
          features: definition.manifest.capabilities.filter((feature) =>
            supportedFeatureCeiling.has(feature),
          ),
        };
        const adapterPackage = packageReference(definition.manifest);

        const nextIdentity = (prefix: string): string => {
          sequence += 1;
          return `acp:${input.instanceId}:${prefix}:${sequence}`;
        };
        const stamp = () => ({ eventId: EventId.make(nextIdentity("event")), createdAt: now() });

        const initialSnapshot: ServerProvider = {
          instanceId: input.instanceId,
          driver: definition.manifest.driver,
          adapterPackage,
          adapterConfigSchema: definition.manifest.configSchema,
          adapterCapabilities: negotiatedCapabilities,
          ...(input.displayName ? { displayName: input.displayName } : {}),
          ...(input.accentColor ? { accentColor: input.accentColor } : {}),
          enabled: input.enabled,
          installed: true,
          version: definition.manifest.version,
          status: input.enabled ? "ready" : "disabled",
          auth: { status: "unknown" },
          checkedAt: now(),
          models: [],
          slashCommands: [],
          skills: [],
        };
        let currentSnapshot = initialSnapshot;

        const publishSnapshotFor = (changedState?: SessionState) =>
          Effect.suspend(() => {
            if (changedState && !changedState.closed) {
              snapshotSequence += 1;
              changedState.snapshotSequence = snapshotSequence;
            }
            let authority: SessionState | undefined;
            for (const state of sessions.values()) {
              if (
                state.closed ||
                (authority && state.snapshotSequence <= authority.snapshotSequence)
              ) {
                continue;
              }
              authority = state;
            }
            const projection = projectAcpConfigOptions(authority?.mapper.configOptions);
            const slashCommands = (authority?.mapper.availableCommands ?? []).flatMap((command) => {
              const name = command.name.trim();
              if (!name) return [];
              const description = command.description.trim();
              const hint = command.input?.hint.trim();
              return [
                {
                  name,
                  ...(description ? { description } : {}),
                  ...(hint ? { input: { hint } } : {}),
                },
              ];
            });
            currentSnapshot = {
              ...currentSnapshot,
              checkedAt: now(),
              models: projection.models,
              slashCommands,
            };
            return PubSub.publish(snapshotChanges, currentSnapshot).pipe(Effect.asVoid);
          });

        // The live event PubSub stays lossless. Local readThread retention is separately bounded and
        // strips raw protocol payloads so slow subscribers do not multiply transcript memory.
        const appendTranscriptItem = (turn: ActiveTurn, item: unknown, terminal = false) => {
          const sanitized = sanitizeTranscriptValue(item);
          if (turn.items.length < MAX_TRANSCRIPT_ITEMS_PER_TURN) {
            turn.items.push(sanitized);
          } else if (terminal) {
            turn.items[MAX_TRANSCRIPT_ITEMS_PER_TURN - 1] = sanitized;
          }
        };
        const transcriptEvent = (event: ProviderRuntimeEvent) => ({
          type: event.type,
          eventId: event.eventId,
          createdAt: event.createdAt,
          provider: event.provider,
          providerInstanceId: event.providerInstanceId,
          adapterPackage: event.adapterPackage,
          threadId: event.threadId,
          ...(event.turnId ? { turnId: event.turnId } : {}),
          ...(event.itemId ? { itemId: event.itemId } : {}),
          ...(event.requestId ? { requestId: event.requestId } : {}),
          payload: sanitizeTranscriptValue(event.payload),
        });
        const recordAndPublish = (state: SessionState, event: ProviderRuntimeEvent) =>
          Effect.gen(function* () {
            const activeTurn = state.activeTurn;
            if (activeTurn && !activeTurn.terminalClaimed) {
              appendTranscriptItem(activeTurn, transcriptEvent(event));
            }
            yield* PubSub.publish(events, event);
          });

        const emitBase = (
          state: SessionState,
          event: Omit<
            ProviderRuntimeEvent,
            | "eventId"
            | "provider"
            | "providerInstanceId"
            | "adapterPackage"
            | "threadId"
            | "createdAt"
          >,
        ) =>
          recordAndPublish(state, {
            ...event,
            ...stamp(),
            provider: definition.manifest.driver,
            providerInstanceId: input.instanceId,
            adapterPackage,
            threadId: state.session.threadId,
          } as ProviderRuntimeEvent);

        const updateSessionFromConfig = (state: SessionState) => {
          const projection = projectAcpConfigOptions(state.mapper.configOptions);
          state.session = {
            ...state.session,
            ...(projection.modelConfig?.currentValue
              ? { model: projection.modelConfig.currentValue }
              : {}),
            updatedAt: now(),
          };
        };

        const emitPermissionResolved = (
          state: SessionState,
          pending: PendingPermission,
          decision: string,
          resolution?: PermissionResponse["outcome"],
        ) =>
          emitBase(state, {
            type: "request.resolved",
            requestId: RuntimeRequestId.make(pending.id),
            ...(state.activeTurn && !state.activeTurn.terminalClaimed
              ? { turnId: state.activeTurn.id }
              : {}),
            payload: {
              requestType: permissionRequestType(pending.request.toolCall.kind),
              decision,
              ...(resolution ? { resolution } : {}),
            },
            raw: {
              source: "acp.jsonrpc",
              method: "session/request_permission",
              payload: pending.request,
            },
          });

        const cancelPendingPermissions = (state: SessionState, decision = "cancel") =>
          Effect.suspend(() => {
            // Claim every entry before yielding so a concurrent UI response cannot resolve it twice.
            const claimed = Array.from(state.pendingPermissions.values());
            state.pendingPermissions.clear();
            return Effect.forEach(
              claimed,
              (pending) =>
                Effect.gen(function* () {
                  const completed = yield* Deferred.succeed(pending.response, {
                    outcome: { outcome: "cancelled" },
                  });
                  if (completed) {
                    yield* emitPermissionResolved(state, pending, decision, {
                      outcome: "cancelled",
                    });
                  }
                }),
              { discard: true },
            );
          });

        const handleSessionUpdate = (notification: AcpSchema.SessionNotification) =>
          Effect.suspend(() => {
            const state = sessionsByAcpId.get(notification.sessionId);
            if (!state || state.closed || state.owner !== liveProcess) return Effect.void;
            return state.mapperLock.withPermits(1)(
              Effect.suspend(() => {
                if (state.closed || state.owner !== liveProcess) return Effect.void;
                const activeTurn = state.activeTurn;
                const result = mapAcpSessionUpdate({
                  state: state.mapper,
                  notification,
                  host: {
                    sessionId: state.acpSessionId,
                    provider: definition.manifest.driver,
                    providerInstanceId: input.instanceId,
                    adapterPackage,
                    threadId: state.session.threadId,
                    ...(activeTurn && !activeTurn.terminalClaimed ? { turnId: activeTurn.id } : {}),
                    stamp: () => stamp(),
                  },
                });
                state.mapper = result.state;
                if (result.snapshotRefreshRequired) updateSessionFromConfig(state);
                return Effect.forEach(result.events, (event) => recordAndPublish(state, event), {
                  discard: true,
                }).pipe(
                  Effect.andThen(
                    result.snapshotRefreshRequired ? publishSnapshotFor(state) : Effect.void,
                  ),
                );
              }),
            );
          });

        const handlePermission = (request: AcpSchema.RequestPermissionRequest) =>
          Effect.gen(function* () {
            const state = sessionsByAcpId.get(request.sessionId);
            if (!state || state.closed || state.owner !== liveProcess) {
              return { outcome: { outcome: "cancelled" } } satisfies PermissionResponse;
            }
            const id = nextIdentity("permission");
            const response = yield* Deferred.make<PermissionResponse>();
            const pending = { id, request, response } satisfies PendingPermission;
            const registered = yield* state.commandLock.withPermits(1)(
              Effect.suspend(() => {
                if (state.closed || state.owner !== liveProcess) return Effect.succeed(false);
                state.pendingPermissions.set(id, pending);
                return emitBase(state, {
                  type: "request.opened",
                  requestId: RuntimeRequestId.make(id),
                  ...(state.activeTurn && !state.activeTurn.terminalClaimed
                    ? { turnId: state.activeTurn.id }
                    : {}),
                  providerRefs: {
                    providerRequestId: id,
                    providerItemId: ProviderItemId.make(request.toolCall.toolCallId),
                  },
                  payload: {
                    requestType: permissionRequestType(request.toolCall.kind),
                    ...(request.toolCall.title ? { detail: request.toolCall.title } : {}),
                    args: { toolCall: request.toolCall, options: request.options },
                  },
                  raw: {
                    source: "acp.jsonrpc",
                    method: "session/request_permission",
                    payload: request,
                  },
                }).pipe(Effect.as(true));
              }),
            );
            if (!registered) {
              return { outcome: { outcome: "cancelled" } } satisfies PermissionResponse;
            }
            return yield* Deferred.await(response).pipe(
              Effect.ensuring(
                Effect.sync(() => {
                  state.pendingPermissions.delete(id);
                }),
              ),
            );
          });

        const finishTurn = (
          state: SessionState,
          turn: ActiveTurn,
          outcome:
            | { readonly state: "completed" | "cancelled"; readonly stopReason?: string | null }
            | { readonly state: "failed"; readonly errorMessage: string },
          rawPayload: unknown,
        ) =>
          Effect.suspend(() => {
            if (turn.terminalClaimed) return Deferred.await(turn.terminalDone);
            if (state.activeTurn !== turn) return Effect.void;
            // This claim happens synchronously before acquiring a lock or publishing anything.
            turn.terminalClaimed = true;
            return Effect.uninterruptible(
              state.mapperLock
                .withPermits(1)(
                  Effect.gen(function* () {
                    const finished = finishAcpTurn({
                      state: state.mapper,
                      host: {
                        sessionId: state.acpSessionId,
                        provider: definition.manifest.driver,
                        providerInstanceId: input.instanceId,
                        adapterPackage,
                        threadId: state.session.threadId,
                        turnId: turn.id,
                        stamp: () => stamp(),
                      },
                      raw: { method: "session/prompt", payload: rawPayload },
                    });
                    state.mapper = finished.state;
                    for (const event of finished.events) {
                      appendTranscriptItem(turn, transcriptEvent(event));
                      yield* PubSub.publish(events, event);
                    }
                    const terminalEvent: ProviderRuntimeEvent = {
                      type: "turn.completed",
                      turnId: turn.id,
                      payload: outcome,
                      raw: { source: "acp.jsonrpc", method: "session/prompt", payload: rawPayload },
                      ...stamp(),
                      provider: definition.manifest.driver,
                      providerInstanceId: input.instanceId,
                      adapterPackage,
                      threadId: state.session.threadId,
                    };
                    appendTranscriptItem(turn, transcriptEvent(terminalEvent), true);
                    state.turns.push({ id: turn.id, items: [...turn.items] });
                    if (state.turns.length > MAX_TRANSCRIPT_TURNS) {
                      state.turns.splice(0, state.turns.length - MAX_TRANSCRIPT_TURNS);
                    }
                    state.activeTurn = undefined;
                    state.session = {
                      ...state.session,
                      status: "ready",
                      activeTurnId: undefined,
                      updatedAt: now(),
                    };
                    yield* PubSub.publish(events, terminalEvent);
                  }),
                )
                .pipe(
                  Effect.ensuring(
                    Deferred.succeed(turn.terminalDone, undefined).pipe(Effect.ignore),
                  ),
                ),
            );
          });

        const clearExitedProcess = (candidate: LiveProcess) =>
          lifecycleLock.withPermits(1)(
            Effect.gen(function* () {
              if (liveProcess === candidate) liveProcess = undefined;
              const exitedSessions = Array.from(sessions.values()).filter(
                (state) => state.owner === candidate,
              );
              for (const state of exitedSessions) {
                yield* state.commandLock.withPermits(1)(
                  Effect.gen(function* () {
                    state.closed = true;
                    const activeTurn = state.activeTurn;
                    if (activeTurn) {
                      yield* finishTurn(
                        state,
                        activeTurn,
                        { state: "failed", errorMessage: "ACP process exited unexpectedly." },
                        { processId: candidate.process.pid },
                      );
                    }
                    yield* cancelPendingPermissions(state);
                    state.closed = true;
                    if (sessions.get(state.session.threadId) === state) {
                      sessions.delete(state.session.threadId);
                    }
                    if (sessionsByAcpId.get(state.acpSessionId) === state) {
                      sessionsByAcpId.delete(state.acpSessionId);
                    }
                  }),
                );
              }
              if (exitedSessions.length > 0) yield* publishSnapshotFor();
            }),
          );

        const spawnProcess = (threadId: ThreadId) => {
          let spawnedProcess: ProviderAdapterProcessV1 | undefined;
          return Effect.gen(function* () {
            const process = yield* host.processes.spawn({
              command: input.config.command,
              ...(input.config.args ? { args: input.config.args } : {}),
              ...(input.config.cwd ? { cwd: input.config.cwd } : {}),
              environment: { ...input.environment, ...input.config.env },
              purpose: { kind: "session", threadId },
            });
            spawnedProcess = process;
            let candidate: LiveProcess | undefined;
            const connection = yield* AcpHostConnection.make(process, {
              onSessionUpdate: handleSessionUpdate,
            });
            yield* connection.client.handleRequestPermission(handlePermission);
            const initialized = yield* connection.client.agent.initialize({
              protocolVersion: 1,
              clientCapabilities: {
                fs: { readTextFile: false, writeTextFile: false },
                terminal: false,
              },
              ...(definition.clientInfo ? { clientInfo: definition.clientInfo } : {}),
            });
            if (initialized.protocolVersion !== 1) {
              return yield* adapterError(
                "initialize",
                `ACP agent negotiated unsupported protocol ${initialized.protocolVersion}.`,
              );
            }
            if (
              negotiatedCapabilities.features.includes("session.resume") &&
              initialized.agentCapabilities?.sessionCapabilities?.resume == null
            ) {
              return yield* adapterError(
                "initialize",
                "ACP adapter manifest requires session.resume but the agent does not advertise it.",
              );
            }
            candidate = {
              process,
              client: connection.client,
              capabilities: initialized.agentCapabilities,
              attachedThreads: new Set([threadId]),
              exited: false,
              closing: false,
            };
            liveProcess = candidate;
            yield* process.exitCode.pipe(
              Effect.matchEffect({
                onFailure: () =>
                  Effect.sync(() => {
                    candidate!.exited = true;
                  }).pipe(Effect.andThen(clearExitedProcess(candidate!))),
                onSuccess: () =>
                  Effect.sync(() => {
                    candidate!.exited = true;
                  }).pipe(Effect.andThen(clearExitedProcess(candidate!))),
              }),
              Effect.forkIn(ownerScope),
            );
            return candidate;
          }).pipe(
            Effect.provideService(Scope.Scope, ownerScope),
            Effect.onError(() =>
              Effect.suspend(() => {
                liveProcess = undefined;
                return spawnedProcess
                  ? spawnedProcess.expectExit.pipe(
                      Effect.andThen(spawnedProcess.close),
                      Effect.ignore,
                    )
                  : Effect.void;
              }),
            ),
          );
        };

        const acquireProcessLocked = (threadId: ThreadId) =>
          Effect.gen(function* () {
            if (!liveProcess) return yield* spawnProcess(threadId);
            if (liveProcess.exited || liveProcess.closing) {
              return yield* adapterError("process", "The ACP process is exiting.");
            }
            if (!liveProcess.attachedThreads.has(threadId)) {
              yield* liveProcess.process.attachSession(threadId);
              liveProcess.attachedThreads.add(threadId);
            }
            return liveProcess;
          });

        const applySelection = (
          state: SessionState,
          client: AcpClient,
          selection: ProviderSendTurnInput["modelSelection"],
        ) =>
          Effect.gen(function* () {
            if (!selection) return;
            let projection = yield* state.mapperLock.withPermits(1)(
              Effect.sync(() => projectAcpConfigOptions(state.mapper.configOptions)),
            );
            const requestedModel = selection.model;
            if (
              !projection.modelConfig ||
              !projection.modelConfig.values.includes(requestedModel)
            ) {
              return yield* adapterError(
                "session/set_config_option",
                `ACP model option '${requestedModel}' is unavailable.`,
              );
            }
            const modelUpdate = getAcpConfigOptionUpdate(projection.modelConfig, requestedModel);
            if (modelUpdate) {
              const result = yield* client.agent.setSessionConfigOption({
                sessionId: state.acpSessionId,
                ...modelUpdate,
              });
              yield* state.mapperLock.withPermits(1)(
                Effect.sync(() => {
                  state.mapper = { ...state.mapper, configOptions: result.configOptions };
                  updateSessionFromConfig(state);
                }),
              );
              yield* publishSnapshotFor(state);
            }

            projection = yield* state.mapperLock.withPermits(1)(
              Effect.sync(() => projectAcpConfigOptions(state.mapper.configOptions)),
            );
            const reasoning = findAcpReasoningSelection(selection.options);
            if (reasoning === undefined) return;
            if (
              !projection.reasoningConfig ||
              !projection.reasoningConfig.values.includes(reasoning)
            ) {
              return yield* adapterError(
                "session/set_config_option",
                `ACP reasoning option '${reasoning}' is unavailable for model '${requestedModel}'.`,
              );
            }
            const reasoningUpdate = getAcpConfigOptionUpdate(projection.reasoningConfig, reasoning);
            if (reasoningUpdate) {
              const result = yield* client.agent.setSessionConfigOption({
                sessionId: state.acpSessionId,
                ...reasoningUpdate,
              });
              yield* state.mapperLock.withPermits(1)(
                Effect.sync(() => {
                  state.mapper = { ...state.mapper, configOptions: result.configOptions };
                  updateSessionFromConfig(state);
                }),
              );
              yield* publishSnapshotFor(state);
            }
          });

        const requireSession = (
          threadId: ThreadId,
          operation: string,
        ): Effect.Effect<SessionState, ProviderAdapterError> => {
          const state = sessions.get(threadId);
          return state && state.ready && !state.closed
            ? Effect.succeed(state)
            : fail(operation, `No active ACP session for thread '${threadId}'.`);
        };

        const startSession = (sessionInput: ProviderSessionStartInput) =>
          protect(
            "startSession",
            lifecycleLock.withPermits(1)(
              (() => {
                let acquiredLive: LiveProcess | undefined;
                let remoteSessionId: string | undefined;
                let registeredState: SessionState | undefined;
                let duplicateSessionId = false;
                const attempt = Effect.gen(function* () {
                  if (sessions.has(sessionInput.threadId)) {
                    return yield* adapterError(
                      "startSession",
                      `Thread '${sessionInput.threadId}' already has an ACP session.`,
                    );
                  }
                  if (
                    sessionInput.resumeCursor !== undefined &&
                    !negotiatedCapabilities.features.includes("session.resume")
                  ) {
                    return yield* adapterError(
                      "session/resume",
                      "ACP adapter manifest does not declare session.resume.",
                    );
                  }
                  const cwd = sessionInput.cwd ?? input.config.cwd;
                  if (!cwd) {
                    return yield* adapterError(
                      "startSession",
                      "ACP session requires a working directory in the session or adapter config.",
                    );
                  }
                  const live = yield* acquireProcessLocked(sessionInput.threadId);
                  acquiredLive = live;
                  let configOptions: ReadonlyArray<AcpSchema.SessionConfigOption> | undefined;
                  if (sessionInput.resumeCursor !== undefined) {
                    if (!supportsResume(live)) {
                      return yield* adapterError(
                        "session/resume",
                        "ACP agent does not advertise session/resume.",
                      );
                    }
                    const cursor = yield* decodeResumeCursor(sessionInput.resumeCursor).pipe(
                      Effect.mapError((cause) =>
                        adapterError("session/resume", "Invalid ACP resume cursor.", cause),
                      ),
                    );
                    const resumed = yield* live.client.agent.resumeSession({
                      sessionId: cursor.sessionId,
                      cwd,
                      mcpServers: input.config.mcpServers ?? [],
                    });
                    remoteSessionId = cursor.sessionId;
                    configOptions = resumed.configOptions ?? undefined;
                  } else {
                    const created = yield* live.client.agent.createSession({
                      cwd,
                      mcpServers: input.config.mcpServers ?? [],
                    });
                    remoteSessionId = created.sessionId;
                    configOptions = created.configOptions ?? undefined;
                  }

                  const existingOwner = sessionsByAcpId.get(remoteSessionId);
                  if (existingOwner && !existingOwner.closed) {
                    duplicateSessionId = true;
                    return yield* adapterError(
                      "startSession",
                      `ACP session '${remoteSessionId}' is already owned by thread '${existingOwner.session.threadId}'.`,
                    );
                  }
                  if (live.exited || live.closing || liveProcess !== live) {
                    return yield* adapterError(
                      "startSession",
                      "ACP process exited before session registration.",
                    );
                  }

                  const commandLock = yield* Semaphore.make(1);
                  const mapperLock = yield* Semaphore.make(1);
                  if (live.exited || live.closing || liveProcess !== live) {
                    return yield* adapterError(
                      "startSession",
                      "ACP process exited before session registration.",
                    );
                  }
                  const createdAt = now();
                  const state: SessionState = {
                    session: {
                      provider: definition.manifest.driver,
                      providerInstanceId: input.instanceId,
                      adapterPackage,
                      status: "ready",
                      runtimeMode: sessionInput.runtimeMode,
                      cwd,
                      threadId: sessionInput.threadId,
                      resumeCursor: {
                        kind: "acp-v1",
                        protocolVersion: 1,
                        sessionId: remoteSessionId,
                      } satisfies AcpResumeCursorV1,
                      createdAt,
                      updatedAt: createdAt,
                    },
                    acpSessionId: remoteSessionId,
                    owner: live,
                    commandLock,
                    mapperLock,
                    mapper: makeAcpEventMapperState(configOptions),
                    turns: [],
                    activeTurn: undefined,
                    pendingPermissions: new Map(),
                    snapshotSequence: 0,
                    ready: false,
                    closed: false,
                  };
                  registeredState = state;
                  sessions.set(sessionInput.threadId, state);
                  sessionsByAcpId.set(remoteSessionId, state);
                  updateSessionFromConfig(state);
                  yield* publishSnapshotFor(state);
                  if (sessionInput.modelSelection) {
                    yield* applySelection(state, live.client, sessionInput.modelSelection);
                  }
                  state.ready = true;
                  return state.session;
                });

                return attempt.pipe(
                  Effect.onError(() =>
                    Effect.gen(function* () {
                      const live = acquiredLive;
                      const state = registeredState;
                      if (state) {
                        state.closed = true;
                        if (sessions.get(sessionInput.threadId) === state) {
                          sessions.delete(sessionInput.threadId);
                        }
                        if (sessionsByAcpId.get(state.acpSessionId) === state) {
                          sessionsByAcpId.delete(state.acpSessionId);
                        }
                      }
                      if (live && remoteSessionId && !duplicateSessionId && supportsClose(live)) {
                        yield* live.client.agent.closeSession({ sessionId: remoteSessionId }).pipe(
                          Effect.ignore,
                          Effect.catchDefect(() => Effect.void),
                        );
                      }
                      if (live?.attachedThreads.delete(sessionInput.threadId)) {
                        yield* live.process.detachSession(sessionInput.threadId).pipe(
                          Effect.ignore,
                          Effect.catchDefect(() => Effect.void),
                        );
                      }
                      if (state) yield* publishSnapshotFor();
                    }),
                  ),
                );
              })(),
            ),
          );

        const sendTurn = (turnInput: ProviderSendTurnInput) =>
          protect(
            "sendTurn",
            Effect.gen(function* () {
              const state = yield* requireSession(turnInput.threadId, "sendTurn");
              if ((turnInput.attachments?.length ?? 0) > 0) {
                return yield* adapterError(
                  "sendTurn",
                  "ACP attachments are not supported by this adapter.",
                );
              }
              if (turnInput.interactionMode === "plan") {
                return yield* adapterError(
                  "sendTurn",
                  "ACP plan interaction mode is not supported by this adapter.",
                );
              }
              if (!turnInput.input) {
                return yield* adapterError("sendTurn", "ACP turns require text input.");
              }
              const promptText = turnInput.input;

              return yield* state.commandLock.withPermits(1)(
                Effect.gen(function* () {
                  const live = state.owner;
                  if (
                    state.closed ||
                    sessions.get(turnInput.threadId) !== state ||
                    liveProcess !== live ||
                    live.exited ||
                    live.closing ||
                    !live.attachedThreads.has(turnInput.threadId)
                  ) {
                    return yield* adapterError("sendTurn", "The ACP session is not available.");
                  }
                  if (state.activeTurn) {
                    return yield* adapterError("sendTurn", "An ACP turn is already active.");
                  }

                  const turnId = TurnId.make(nextIdentity("turn"));
                  const terminalDone = yield* Deferred.make<void>();
                  const turn: ActiveTurn = {
                    id: turnId,
                    items: [],
                    cancelled: false,
                    terminalClaimed: false,
                    terminalDone,
                  };
                  appendTranscriptItem(turn, {
                    type: "userMessage",
                    content: [{ type: "text", text: promptText }],
                  });
                  // Reserve the turn before model RPCs so another send cannot pass validation.
                  state.activeTurn = turn;
                  state.session = {
                    ...state.session,
                    status: "running",
                    activeTurnId: turnId,
                    updatedAt: now(),
                  };

                  const configureAndStart = Effect.gen(function* () {
                    yield* applySelection(state, live.client, turnInput.modelSelection);
                    const projection = projectAcpConfigOptions(state.mapper.configOptions);
                    yield* emitBase(state, {
                      type: "turn.started",
                      turnId,
                      payload: {
                        ...(projection.modelConfig?.currentValue
                          ? { model: projection.modelConfig.currentValue }
                          : {}),
                        ...(projection.reasoningConfig?.currentValue
                          ? { effort: projection.reasoningConfig.currentValue }
                          : {}),
                      },
                    });

                    yield* Effect.suspend(() => {
                      if (
                        state.closed ||
                        state.activeTurn !== turn ||
                        turn.terminalClaimed ||
                        live.exited ||
                        live.closing
                      ) {
                        return Effect.void;
                      }
                      return live.client.agent
                        .prompt({
                          sessionId: state.acpSessionId,
                          prompt: [{ type: "text", text: promptText }],
                        })
                        .pipe(
                          Effect.matchEffect({
                            onFailure: (cause) =>
                              finishTurn(
                                state,
                                turn,
                                turn.cancelled
                                  ? { state: "cancelled", stopReason: "cancelled" }
                                  : { state: "failed", errorMessage: detailOf(cause) },
                                cause,
                              ),
                            onSuccess: (response) =>
                              finishTurn(
                                state,
                                turn,
                                turn.cancelled || response.stopReason === "cancelled"
                                  ? { state: "cancelled", stopReason: response.stopReason }
                                  : { state: "completed", stopReason: response.stopReason },
                                response,
                              ),
                          }),
                        );
                    }).pipe(Effect.forkIn(ownerScope));
                    return {
                      threadId: state.session.threadId,
                      turnId,
                      resumeCursor: state.session.resumeCursor,
                    };
                  });

                  return yield* configureAndStart.pipe(
                    Effect.onError(() =>
                      Effect.sync(() => {
                        if (state.activeTurn === turn && !turn.terminalClaimed) {
                          state.activeTurn = undefined;
                          state.session = {
                            ...state.session,
                            status: "ready",
                            activeTurnId: undefined,
                            updatedAt: now(),
                          };
                        }
                      }),
                    ),
                  );
                }),
              );
            }),
          );

        const interruptTurn = (threadId: ThreadId, turnId?: TurnId) =>
          protect(
            "interruptTurn",
            Effect.gen(function* () {
              const state = yield* requireSession(threadId, "interruptTurn");
              return yield* state.commandLock.withPermits(1)(
                Effect.gen(function* () {
                  if (state.closed || sessions.get(threadId) !== state) {
                    return yield* adapterError("interruptTurn", "The ACP session is unavailable.");
                  }
                  if (turnId !== undefined && state.activeTurn?.id !== turnId) {
                    return yield* adapterError(
                      "interruptTurn",
                      `Turn '${turnId}' is not active for thread '${threadId}'.`,
                    );
                  }
                  if (state.activeTurn) state.activeTurn.cancelled = true;
                  return yield* state.owner.client.agent
                    .cancel({ sessionId: state.acpSessionId })
                    .pipe(Effect.ensuring(cancelPendingPermissions(state)));
                }),
              );
            }),
          );

        const respondToRequest = (
          threadId: ThreadId,
          requestId: ApprovalRequestId,
          decision: ProviderApprovalDecision,
        ) =>
          protect(
            "respondToRequest",
            Effect.gen(function* () {
              const state = yield* requireSession(threadId, "respondToRequest");
              return yield* state.commandLock.withPermits(1)(
                Effect.gen(function* () {
                  const pending = state.pendingPermissions.get(String(requestId));
                  if (!pending) {
                    return yield* adapterError(
                      "respondToRequest",
                      `Permission request '${requestId}' is not pending.`,
                    );
                  }
                  let outcome: PermissionResponse["outcome"];
                  if (decision === "cancel") {
                    outcome = { outcome: "cancelled" };
                  } else {
                    const kind = permissionKindForDecision(decision)!;
                    const option = pending.request.options.find(
                      (candidate) => candidate.kind === kind,
                    );
                    if (!option) {
                      return yield* adapterError(
                        "respondToRequest",
                        `ACP permission request has no '${kind}' option for decision '${decision}'.`,
                      );
                    }
                    outcome = { outcome: "selected", optionId: option.optionId };
                  }
                  // Claim before yielding. A second response or teardown now observes no entry.
                  state.pendingPermissions.delete(pending.id);
                  const completed = yield* Deferred.succeed(pending.response, { outcome });
                  if (!completed) {
                    return yield* adapterError(
                      "respondToRequest",
                      `Permission request '${requestId}' was already resolved.`,
                    );
                  }
                  yield* emitPermissionResolved(state, pending, decision, outcome);
                }),
              );
            }),
          );

        const stopSession = (threadId: ThreadId) =>
          protect(
            "stopSession",
            lifecycleLock.withPermits(1)(
              Effect.gen(function* () {
                const state = yield* requireSession(threadId, "stopSession");
                return yield* state.commandLock.withPermits(1)(
                  Effect.suspend(() => {
                    if (state.closed || sessions.get(threadId) !== state) return Effect.void;
                    state.closed = true;
                    const live = state.owner;
                    const activeTurn = state.activeTurn;
                    let remoteError: unknown | undefined;

                    const cleanup = Effect.gen(function* () {
                      if (sessions.get(threadId) === state) sessions.delete(threadId);
                      if (sessionsByAcpId.get(state.acpSessionId) === state) {
                        sessionsByAcpId.delete(state.acpSessionId);
                      }
                      if (live.attachedThreads.delete(threadId)) {
                        yield* live.process.detachSession(threadId).pipe(
                          Effect.ignore,
                          Effect.catchDefect(() => Effect.void),
                        );
                      }
                      yield* publishSnapshotFor();
                    });

                    const shutdown = Effect.gen(function* () {
                      if (activeTurn) {
                        activeTurn.cancelled = true;
                        yield* live.client.agent.cancel({ sessionId: state.acpSessionId }).pipe(
                          Effect.catch((error) =>
                            Effect.sync(() => {
                              remoteError = error;
                            }),
                          ),
                        );
                      }
                      yield* cancelPendingPermissions(state);
                      if (activeTurn) {
                        yield* finishTurn(
                          state,
                          activeTurn,
                          { state: "cancelled", stopReason: "session stopped" },
                          { stopReason: "session stopped" },
                        );
                      }
                      if (supportsClose(live)) {
                        yield* live.client.agent
                          .closeSession({ sessionId: state.acpSessionId })
                          .pipe(
                            Effect.catch((error) =>
                              Effect.sync(() => {
                                remoteError ??= error;
                              }),
                            ),
                          );
                      }
                      if (remoteError !== undefined) {
                        return yield* adapterError(
                          "stopSession.remote",
                          detailOf(remoteError),
                          remoteError,
                        );
                      }
                    });
                    return shutdown.pipe(Effect.ensuring(cleanup));
                  }),
                );
              }),
            ),
          );

        const stopAll = () =>
          protect(
            "stopAll",
            lifecycleLock.withPermits(1)(
              Effect.gen(function* () {
                const live = liveProcess;
                if (live) live.closing = true;
                liveProcess = undefined;
                const allSessions = Array.from(sessions.values());
                for (const state of allSessions) {
                  yield* state.commandLock.withPermits(1)(
                    Effect.gen(function* () {
                      state.closed = true;
                      const activeTurn = state.activeTurn;
                      if (activeTurn) {
                        activeTurn.cancelled = true;
                        yield* state.owner.client.agent
                          .cancel({ sessionId: state.acpSessionId })
                          .pipe(Effect.ignore);
                      }
                      yield* cancelPendingPermissions(state);
                      if (activeTurn) {
                        yield* finishTurn(
                          state,
                          activeTurn,
                          { state: "cancelled", stopReason: "adapter stopped" },
                          { stopReason: "adapter stopped" },
                        );
                      }
                      state.closed = true;
                      if (sessions.get(state.session.threadId) === state) {
                        sessions.delete(state.session.threadId);
                      }
                      if (sessionsByAcpId.get(state.acpSessionId) === state) {
                        sessionsByAcpId.delete(state.acpSessionId);
                      }
                    }),
                  );
                }
                yield* publishSnapshotFor();
                if (!live) return;
                yield* live.process.expectExit;
                yield* live.process.close;
              }),
            ),
          );

        const unsupported = (operation: string) =>
          fail(operation, `ACP ${operation} is not supported by this adapter.`);

        return {
          snapshot: {
            getSnapshot: Effect.sync(() => currentSnapshot),
            refresh: Effect.sync(() => currentSnapshot),
            streamChanges: Stream.fromPubSub(snapshotChanges),
          },
          adapter: {
            capabilities: negotiatedCapabilities,
            startSession,
            sendTurn,
            interruptTurn,
            respondToRequest,
            respondToUserInput: () => unsupported("respondToUserInput"),
            stopSession,
            listSessions: () =>
              Effect.sync(() =>
                Array.from(sessions.values())
                  .filter((state) => state.ready && !state.closed)
                  .map((state) => state.session),
              ),
            hasSession: (threadId: ThreadId) =>
              Effect.sync(() => {
                const state = sessions.get(threadId);
                return state !== undefined && state.ready && !state.closed;
              }),
            readThread: (threadId: ThreadId) =>
              requireSession(threadId, "readThread").pipe(
                Effect.map((state) => ({
                  threadId,
                  turns: state.turns.map((turn) => ({ ...turn, items: [...turn.items] })),
                })),
              ),
            rollbackThread: () => unsupported("rollbackThread"),
            stopAll,
            streamEvents: Stream.fromPubSub(events),
          },
        };
      }).pipe(
        Effect.mapError((cause) =>
          isProviderAdapterError(cause) ? cause : adapterError("create", detailOf(cause), cause),
        ),
      ),
  });
