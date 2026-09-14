/**
 * Author-facing interface for trusted local T3 provider adapter packages.
 *
 * V1 keeps executable package code on the server. The host owns process
 * spawning and teardown, while the package translates its harness protocol
 * into canonical provider sessions, commands, and runtime events.
 */
import type {
  ApprovalRequestId,
  ProviderAdapterManifestV1,
  ProviderAdapterProtocolCapabilitiesV1,
  ProviderApprovalDecision,
  ProviderInstanceId,
  ProviderRuntimeEvent,
  ProviderSendTurnInput,
  ProviderSession,
  ProviderSessionStartInput,
  ProviderTurnStartResult,
  ProviderUploadFeedbackInput,
  ProviderUploadFeedbackResult,
  ProviderUserInputAnswers,
  ServerProvider,
  ThreadId,
  TurnId,
} from "@t3tools/contracts";
import type * as Effect from "effect/Effect";
import * as Schema from "effect/Schema";
import type * as Scope from "effect/Scope";
import type * as Stream from "effect/Stream";

export {
  ProviderAdapterManifestV1,
  ProviderAdapterPackageId,
  ProviderAdapterPackageVersion,
  ProviderAdapterProtocolCapabilitiesV1,
  T3_PROVIDER_ADAPTER_PROTOCOL_VERSION,
} from "@t3tools/contracts";

export class ProviderAdapterV1Error extends Schema.TaggedErrorClass<ProviderAdapterV1Error>()(
  "ProviderAdapterV1Error",
  {
    operation: Schema.String,
    detail: Schema.String,
    cause: Schema.optional(Schema.Defect()),
  },
) {
  override get message(): string {
    return `External provider adapter failed in ${this.operation}: ${this.detail}`;
  }
}

export class ProviderAdapterHostProcessError extends Schema.TaggedErrorClass<ProviderAdapterHostProcessError>()(
  "ProviderAdapterHostProcessError",
  {
    operation: Schema.String,
    detail: Schema.String,
    cause: Schema.optional(Schema.Defect()),
  },
) {
  override get message(): string {
    return `Host-supervised adapter process failed in ${this.operation}: ${this.detail}`;
  }
}

export interface ProviderAdapterProcessSpawnV1 {
  readonly command: string;
  readonly args?: ReadonlyArray<string> | undefined;
  readonly cwd?: string | undefined;
  readonly environment?: Readonly<Record<string, string | undefined>> | undefined;
}

export interface ProviderAdapterProcessV1 {
  readonly pid: number;
  readonly stdout: Stream.Stream<Uint8Array, ProviderAdapterHostProcessError>;
  readonly stderr: Stream.Stream<Uint8Array, ProviderAdapterHostProcessError>;
  readonly exitCode: Effect.Effect<number, ProviderAdapterHostProcessError>;
  readonly write: (chunk: Uint8Array) => Effect.Effect<void, ProviderAdapterHostProcessError>;
  readonly close: Effect.Effect<void>;
}

export interface ProviderAdapterProcessSupervisorV1 {
  readonly spawn: (
    input: ProviderAdapterProcessSpawnV1,
  ) => Effect.Effect<ProviderAdapterProcessV1, ProviderAdapterHostProcessError, Scope.Scope>;
}

export interface ProviderAdapterHostV1 {
  readonly protocolVersion: 1;
  readonly processes: ProviderAdapterProcessSupervisorV1;
}

export interface ProviderAdapterThreadTurnSnapshotV1 {
  readonly id: TurnId;
  readonly items: ReadonlyArray<unknown>;
}

export interface ProviderAdapterThreadSnapshotV1 {
  readonly threadId: ThreadId;
  readonly turns: ReadonlyArray<ProviderAdapterThreadTurnSnapshotV1>;
}

/** Canonical command/event interface. Optional harness behavior is capability-gated. */
export interface ProviderAdapterV1 {
  readonly capabilities: ProviderAdapterProtocolCapabilitiesV1;
  readonly startSession: (
    input: ProviderSessionStartInput,
  ) => Effect.Effect<ProviderSession, ProviderAdapterV1Error>;
  readonly sendTurn: (
    input: ProviderSendTurnInput,
  ) => Effect.Effect<ProviderTurnStartResult, ProviderAdapterV1Error>;
  readonly interruptTurn: (
    threadId: ThreadId,
    turnId?: TurnId,
  ) => Effect.Effect<void, ProviderAdapterV1Error>;
  readonly respondToRequest: (
    threadId: ThreadId,
    requestId: ApprovalRequestId,
    decision: ProviderApprovalDecision,
  ) => Effect.Effect<void, ProviderAdapterV1Error>;
  readonly respondToUserInput: (
    threadId: ThreadId,
    requestId: ApprovalRequestId,
    answers: ProviderUserInputAnswers,
  ) => Effect.Effect<void, ProviderAdapterV1Error>;
  readonly stopSession: (threadId: ThreadId) => Effect.Effect<void, ProviderAdapterV1Error>;
  readonly listSessions: () => Effect.Effect<ReadonlyArray<ProviderSession>>;
  readonly hasSession: (threadId: ThreadId) => Effect.Effect<boolean>;
  readonly readThread: (
    threadId: ThreadId,
  ) => Effect.Effect<ProviderAdapterThreadSnapshotV1, ProviderAdapterV1Error>;
  readonly rollbackThread: (
    threadId: ThreadId,
    numTurns: number,
  ) => Effect.Effect<ProviderAdapterThreadSnapshotV1, ProviderAdapterV1Error>;
  readonly uploadFeedback?:
    | ((
        input: ProviderUploadFeedbackInput,
      ) => Effect.Effect<ProviderUploadFeedbackResult, ProviderAdapterV1Error>)
    | undefined;
  readonly stopAll: () => Effect.Effect<void, ProviderAdapterV1Error>;
  readonly streamEvents: Stream.Stream<ProviderRuntimeEvent>;
}

export interface ProviderAdapterSnapshotV1 {
  readonly getSnapshot: Effect.Effect<ServerProvider>;
  readonly refresh: Effect.Effect<ServerProvider>;
  readonly streamChanges: Stream.Stream<ServerProvider>;
}

export interface ProviderAdapterInstanceV1 {
  readonly continuationKey?: string | undefined;
  readonly snapshot: ProviderAdapterSnapshotV1;
  readonly adapter: ProviderAdapterV1;
}

export interface ProviderAdapterCreateInputV1<Config> {
  readonly instanceId: ProviderInstanceId;
  readonly displayName: string | undefined;
  readonly accentColor: string | undefined;
  readonly environment: Readonly<Record<string, string | undefined>>;
  readonly enabled: boolean;
  readonly config: Config;
}

export interface ProviderAdapterPackageV1<Config = unknown> {
  readonly manifest: ProviderAdapterManifestV1;
  readonly configSchema: Schema.Codec<Config, unknown>;
  readonly defaultConfig: () => Config;
  readonly create: (
    input: ProviderAdapterCreateInputV1<Config>,
    host: ProviderAdapterHostV1,
  ) => Effect.Effect<ProviderAdapterInstanceV1, ProviderAdapterV1Error, Scope.Scope>;
}

/** Identity helper that preserves the package's concrete config type. */
export const defineProviderAdapterV1 = <Config>(
  adapterPackage: ProviderAdapterPackageV1<Config>,
): ProviderAdapterPackageV1<Config> => adapterPackage;
