/**
 * Author-facing interface for trusted local T3 provider adapter packages.
 *
 * V1 keeps executable package code on the server. The host owns process
 * spawning and teardown, while the package translates its harness protocol
 * into canonical provider sessions, commands, and runtime events.
 */
import type {
  ApprovalRequestId,
  ChatAttachment,
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
import { T3_PROVIDER_ADAPTER_HOST_PROTOCOL_VERSION } from "@t3tools/contracts";
import type * as Effect from "effect/Effect";
import * as Schema from "effect/Schema";
import type * as Scope from "effect/Scope";
import type * as Stream from "effect/Stream";

export {
  ProviderAdapterManifestV1,
  ProviderAdapterPackageId,
  ProviderAdapterPackageVersion,
  ProviderAdapterProtocolCapabilitiesV1,
  T3_PROVIDER_ADAPTER_HOST_PROTOCOL_VERSION,
  T3_PROVIDER_ADAPTER_PROTOCOL_VERSION,
  T3_PROVIDER_ADAPTER_SUPPORTED_HOST_PROTOCOL_VERSIONS,
} from "@t3tools/contracts";

export class ProviderAdapterV1Error extends Schema.TaggedError<ProviderAdapterV1Error>()(
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

export class ProviderAdapterHostProcessError extends Schema.TaggedError<ProviderAdapterHostProcessError>()(
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

export const ProviderAdapterHostResourceOperation = Schema.Literals([
  "workspaces.resolveCwd",
  "storage.configure",
  "storage.prepareSession",
  "storage.validateSessionFile",
  "storage.materializeArtifact",
  "attachments.read",
]);
export type ProviderAdapterHostResourceOperation = typeof ProviderAdapterHostResourceOperation.Type;

export class ProviderAdapterHostResourceError extends Schema.TaggedError<ProviderAdapterHostResourceError>()(
  "ProviderAdapterHostResourceError",
  {
    operation: ProviderAdapterHostResourceOperation,
    detail: Schema.String,
    cause: Schema.optional(Schema.Defect()),
  },
) {
  override get message(): string {
    return `Host adapter resource failed in ${this.operation}: ${this.detail}`;
  }
}

export type ProviderAdapterProcessPurposeV1 =
  | { readonly kind: "probe" }
  | { readonly kind: "session"; readonly threadId: ThreadId };

export interface ProviderAdapterProcessSpawnV1 {
  readonly command: string;
  readonly args?: ReadonlyArray<string> | undefined;
  readonly cwd?: string | undefined;
  readonly environment?: Readonly<Record<string, string | undefined>> | undefined;
  /** Required ownership lets the host turn unexpected harness exits into canonical failures. */
  readonly purpose: ProviderAdapterProcessPurposeV1;
}

export interface ProviderAdapterProcessV1 {
  readonly pid: number;
  /** Link another session when one multiplexed process serves several threads. */
  readonly attachSession: (threadId: ThreadId) => Effect.Effect<void>;
  readonly detachSession: (threadId: ThreadId) => Effect.Effect<void>;
  /** Mark the harness's next natural exit as expected without terminating it. */
  readonly expectExit: Effect.Effect<void>;
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

export interface ProviderAdapterSessionStorageV2 {
  readonly sessionDirectory: string;
  readonly sharedDirectory: string;
}

export interface ProviderAdapterValidateSessionFileInputV2 {
  readonly threadId: ThreadId;
  readonly path: string;
  readonly mustExist: boolean;
}

export interface ProviderAdapterMaterializeArtifactInputV2 {
  readonly key: string;
  readonly fileName: string;
  readonly content: string | Uint8Array;
}

export interface ProviderAdapterAttachmentReadResultV2 {
  readonly bytes: Uint8Array;
  readonly path: string;
}

export interface ProviderAdapterWorkspacesV2 {
  readonly resolveCwd: (
    requested?: string,
  ) => Effect.Effect<string, ProviderAdapterHostResourceError>;
}

export interface ProviderAdapterStorageV2 {
  readonly prepareSession: (
    threadId: ThreadId,
  ) => Effect.Effect<ProviderAdapterSessionStorageV2, ProviderAdapterHostResourceError>;
  readonly validateSessionFile: (
    input: ProviderAdapterValidateSessionFileInputV2,
  ) => Effect.Effect<string, ProviderAdapterHostResourceError>;
  readonly materializeArtifact: (
    input: ProviderAdapterMaterializeArtifactInputV2,
  ) => Effect.Effect<string, ProviderAdapterHostResourceError>;
}

export interface ProviderAdapterAttachmentsV2 {
  readonly read: (
    attachment: ChatAttachment,
  ) => Effect.Effect<ProviderAdapterAttachmentReadResultV2, ProviderAdapterHostResourceError>;
}

export interface ProviderAdapterHostV2 {
  readonly protocolVersion: typeof T3_PROVIDER_ADAPTER_HOST_PROTOCOL_VERSION;
  readonly processes: ProviderAdapterProcessSupervisorV1;
  readonly workspaces: ProviderAdapterWorkspacesV2;
  readonly storage: ProviderAdapterStorageV2;
  readonly attachments: ProviderAdapterAttachmentsV2;
}

export type ProviderAdapterHost = ProviderAdapterHostV1 | ProviderAdapterHostV2;

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

export interface ProviderAdapterPackageV1<
  Config = unknown,
  Host extends ProviderAdapterHost = ProviderAdapterHostV1,
> {
  readonly manifest: ProviderAdapterManifestV1;
  /** Optional server-only storage key. This is not exposed through the client manifest. */
  readonly storageKey?: string | undefined;
  readonly configSchema: Schema.Codec<Config, unknown>;
  readonly defaultConfig: () => Config;
  readonly create: (
    input: ProviderAdapterCreateInputV1<Config>,
    host: Host,
  ) => Effect.Effect<ProviderAdapterInstanceV1, ProviderAdapterV1Error, Scope.Scope>;
}

/** Identity helper that preserves the package's concrete config and required host type. */
export const defineProviderAdapterV1 = <
  Config,
  Host extends ProviderAdapterHost = ProviderAdapterHostV1,
>(
  adapterPackage: ProviderAdapterPackageV1<Config, Host>,
): ProviderAdapterPackageV1<Config, Host> => adapterPackage;
