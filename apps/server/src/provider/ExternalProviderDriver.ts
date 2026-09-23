import * as NodeCrypto from "node:crypto";
import {
  T3_PROVIDER_ADAPTER_SUPPORTED_HOST_PROTOCOL_VERSIONS,
  type ProviderAdapterHost,
  type ProviderAdapterHostV1,
  type ProviderAdapterInstanceV1,
  type ProviderAdapterPackageV1,
} from "@t3tools/provider-adapter";
import {
  EventId,
  ProviderAdapterProtocolCapabilitiesV1,
  ProviderRuntimeEvent,
  ProviderSession,
  ProviderTurnStartResult,
  ServerProvider,
  ThreadId,
  TextGenerationError,
  TurnId,
  type ProviderAdapterPackageReference,
  type ProviderAdapterProtocolCapabilitiesV1 as ProviderAdapterCapabilities,
} from "@t3tools/contracts";
import * as Cause from "effect/Cause";
import * as DateTime from "effect/DateTime";
import * as Effect from "effect/Effect";
import * as FileSystem from "effect/FileSystem";
import * as Path from "effect/Path";
import * as Schema from "effect/Schema";
import * as Stream from "effect/Stream";
import * as ChildProcessSpawner from "effect/unstable/process/ChildProcessSpawner";
import { HttpClient } from "effect/unstable/http";

import { ServerConfig } from "../config.ts";
import { ServerSettingsService } from "../serverSettings.ts";
import { ProviderAdapterRequestError, ProviderDriverError } from "./Errors.ts";
import { makeExternalProviderAdapterHostV2 } from "./ExternalProviderAdapterHost.ts";
import {
  defaultProviderContinuationIdentity,
  type AnyProviderDriver,
  type ProviderInstance,
} from "./ProviderDriver.ts";
import { mergeProviderInstanceEnvironment } from "./ProviderInstanceEnvironment.ts";
import {
  enrichProviderSnapshotWithVersionAdvisory,
  makeCachedProviderMaintenanceResolution,
  makeManualOnlyProviderMaintenanceCapabilities,
  makePackageManagedProviderMaintenanceResolver,
  resolveProviderMaintenanceCapabilitiesEffect,
} from "./providerMaintenance.ts";
import { buildUnavailableProviderSnapshot } from "./unavailableProviderSnapshot.ts";

export type ExternalProviderDriverEnv =
  | ChildProcessSpawner.ChildProcessSpawner
  | FileSystem.FileSystem
  | Path.Path
  | HttpClient.HttpClient
  | ServerConfig
  | ServerSettingsService;

const packageReference = <Config, Host extends ProviderAdapterHost>(
  adapterPackage: ProviderAdapterPackageV1<Config, Host>,
): ProviderAdapterPackageReference => ({
  id: adapterPackage.manifest.id,
  version: adapterPackage.manifest.version,
  protocolVersion: adapterPackage.manifest.protocolVersion,
});

const adapterFailure = (
  provider: string,
  operation: string,
  error: unknown,
): ProviderAdapterRequestError =>
  new ProviderAdapterRequestError({
    provider,
    method: operation,
    detail:
      typeof error === "object" && error !== null && "detail" in error
        ? String(error.detail)
        : String(error),
    cause: error,
  });

const protectAdapterEffect = <A, E>(
  provider: string,
  operation: string,
  effect: Effect.Effect<A, E>,
): Effect.Effect<A, ProviderAdapterRequestError> =>
  effect.pipe(
    Effect.mapError((error) => adapterFailure(provider, operation, error)),
    Effect.catchDefect((defect) => Effect.fail(adapterFailure(provider, operation, defect))),
  );

const makeUnsupportedTextGeneration = (provider: string): ProviderInstance["textGeneration"] => {
  const unsupported = (operation: string) =>
    Effect.fail(
      new TextGenerationError({
        operation,
        detail: `External provider '${provider}' does not implement host text generation.`,
      }),
    );
  return {
    generateCommitMessage: () => unsupported("generateCommitMessage"),
    generatePrContent: () => unsupported("generatePrContent"),
    generateBranchName: () => unsupported("generateBranchName"),
    generateThreadTitle: () => unsupported("generateThreadTitle"),
  };
};

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const requiredAdapterMethods = [
  "startSession",
  "sendTurn",
  "interruptTurn",
  "respondToRequest",
  "respondToUserInput",
  "stopSession",
  "listSessions",
  "hasSession",
  "readThread",
  "rollbackThread",
  "stopAll",
] as const;

const decodeCapabilities = Schema.decodeUnknownExit(ProviderAdapterProtocolCapabilitiesV1);
const decodeProviderSession = Schema.decodeUnknownEffect(ProviderSession);
const decodeProviderSessions = Schema.decodeUnknownEffect(Schema.Array(ProviderSession));
const decodeTurnStartResult = Schema.decodeUnknownEffect(ProviderTurnStartResult);
const decodeRuntimeEvent = Schema.decodeUnknownEffect(ProviderRuntimeEvent);
const decodeBoolean = Schema.decodeUnknownEffect(Schema.Boolean);
const decodeServerProvider = Schema.decodeUnknownEffect(ServerProvider);
const ProviderAdapterThreadSnapshot = Schema.Struct({
  threadId: ThreadId,
  turns: Schema.Array(
    Schema.Struct({
      id: TurnId,
      items: Schema.Array(Schema.Unknown),
    }),
  ),
});
const decodeThreadSnapshot = Schema.decodeUnknownEffect(ProviderAdapterThreadSnapshot);

const decodeAdapterInstance = <Config, Host extends ProviderAdapterHost>(
  value: unknown,
  adapterPackage: ProviderAdapterPackageV1<Config, Host>,
  instanceId: ProviderInstance["instanceId"],
): Effect.Effect<
  {
    readonly instance: ProviderAdapterInstanceV1;
    readonly capabilities: ProviderAdapterCapabilities;
  },
  ProviderDriverError
> => {
  if (!isRecord(value) || !isRecord(value.snapshot) || !isRecord(value.adapter)) {
    return Effect.fail(
      new ProviderDriverError({
        driver: adapterPackage.manifest.driver,
        instanceId,
        detail: "Adapter create returned an invalid V1 instance.",
      }),
    );
  }
  const { snapshot, adapter } = value;
  const shapeValid =
    Effect.isEffect(snapshot.getSnapshot) &&
    Effect.isEffect(snapshot.refresh) &&
    Stream.isStream(snapshot.streamChanges) &&
    requiredAdapterMethods.every((method) => typeof adapter[method] === "function") &&
    (adapter.uploadFeedback === undefined || typeof adapter.uploadFeedback === "function") &&
    Stream.isStream(adapter.streamEvents) &&
    (value.continuationKey === undefined || typeof value.continuationKey === "string");
  const capabilitiesResult = decodeCapabilities(adapter.capabilities);
  if (!shapeValid || capabilitiesResult._tag === "Failure") {
    return Effect.fail(
      new ProviderDriverError({
        driver: adapterPackage.manifest.driver,
        instanceId,
        detail: "Adapter create returned an invalid V1 instance.",
      }),
    );
  }
  const declared = new Set(adapterPackage.manifest.capabilities);
  const undeclared = capabilitiesResult.value.features.filter((feature) => !declared.has(feature));
  if (undeclared.length > 0) {
    return Effect.fail(
      new ProviderDriverError({
        driver: adapterPackage.manifest.driver,
        instanceId,
        detail: `Adapter negotiated undeclared capabilities: ${undeclared.join(", ")}.`,
      }),
    );
  }
  return Effect.succeed({
    instance: value as unknown as ProviderAdapterInstanceV1,
    capabilities: capabilitiesResult.value,
  });
};

type SupportedHostProtocolVersion =
  (typeof T3_PROVIDER_ADAPTER_SUPPORTED_HOST_PROTOCOL_VERSIONS)[number];

const negotiateHostProtocolVersion = (
  minimum: number,
  maximum: number,
): SupportedHostProtocolVersion | undefined => {
  let negotiated: SupportedHostProtocolVersion | undefined;
  for (const version of T3_PROVIDER_ADAPTER_SUPPORTED_HOST_PROTOCOL_VERSIONS) {
    if (
      version >= minimum &&
      version <= maximum &&
      (negotiated === undefined || version > negotiated)
    ) {
      negotiated = version;
    }
  }
  return negotiated;
};

export const makeExternalProviderDriver = <Config, Host extends ProviderAdapterHost>(
  adapterPackage: ProviderAdapterPackageV1<Config, Host>,
  defaultConfig: Config,
): AnyProviderDriver<ExternalProviderDriverEnv> => {
  const provider = adapterPackage.manifest.driver;
  const adapterPackageRef = packageReference(adapterPackage);

  return {
    driverKind: provider,
    adapterPackage: adapterPackageRef,
    metadata: {
      displayName: adapterPackage.manifest.displayName,
      supportsMultipleInstances: true,
    },
    configSchema: adapterPackage.configSchema,
    defaultConfig: () => defaultConfig,
    create: ({ instanceId, displayName, accentColor, environment, enabled, config }) =>
      Effect.gen(function* () {
        const spawner = yield* ChildProcessSpawner.ChildProcessSpawner;
        const fileSystem = yield* FileSystem.FileSystem;
        const pathService = yield* Path.Path;
        const processEnv = mergeProviderInstanceEnvironment(environment);
        const negotiatedProtocolVersion = negotiateHostProtocolVersion(
          adapterPackage.manifest.hostProtocol.minimum,
          adapterPackage.manifest.hostProtocol.maximum,
        );
        if (negotiatedProtocolVersion === undefined) {
          return yield* new ProviderDriverError({
            driver: provider,
            instanceId,
            detail: `Adapter does not support a host protocol available in this server.`,
          });
        }

        const currentHost = yield* makeExternalProviderAdapterHostV2({
          packageId: adapterPackage.manifest.id,
          instanceId,
          storageKey: adapterPackage.storageKey,
        }).pipe(
          Effect.mapError(
            (error) =>
              new ProviderDriverError({
                driver: provider,
                instanceId,
                detail: error.message,
                cause: error,
              }),
          ),
          Effect.catchDefect((defect) =>
            Effect.fail(
              new ProviderDriverError({
                driver: provider,
                instanceId,
                detail: `Adapter host creation defected: ${String(defect)}`,
                cause: defect,
              }),
            ),
          ),
        );
        const negotiatedHost: ProviderAdapterHost =
          negotiatedProtocolVersion === 1
            ? ({
                protocolVersion: 1,
                processes: currentHost.host.processes,
              } satisfies ProviderAdapterHostV1)
            : currentHost.host;

        const created = yield* Effect.suspend(() =>
          adapterPackage.create(
            {
              instanceId,
              displayName,
              accentColor,
              environment: processEnv,
              enabled,
              config,
            },
            // The package author ties its concrete Host type to this runtime manifest.
            // TypeScript cannot express that relationship across the manifest boundary.
            negotiatedHost as Host,
          ),
        ).pipe(
          Effect.mapError(
            (error) =>
              new ProviderDriverError({
                driver: provider,
                instanceId,
                detail: error.message,
                cause: error,
              }),
          ),
          Effect.catchDefect((defect) =>
            Effect.fail(
              new ProviderDriverError({
                driver: provider,
                instanceId,
                detail: `Adapter create defected: ${String(defect)}`,
                cause: defect,
              }),
            ),
          ),
        );
        const { instance: external, capabilities } = yield* decodeAdapterInstance(
          created,
          adapterPackage,
          instanceId,
        );

        const continuationIdentity = external.continuationKey
          ? { driverKind: provider, continuationKey: external.continuationKey }
          : defaultProviderContinuationIdentity({ driverKind: provider, instanceId });
        const withSessionIdentity = (session: ProviderSession): ProviderSession => ({
          ...session,
          provider,
          providerInstanceId: instanceId,
          adapterPackage: adapterPackageRef,
        });
        const withSnapshotIdentity = (snapshot: ServerProvider): ServerProvider => ({
          ...snapshot,
          instanceId,
          driver: provider,
          adapterPackage: adapterPackageRef,
          adapterConfigSchema: adapterPackage.manifest.configSchema,
          adapterCapabilities: capabilities,
          ...(displayName ? { displayName } : {}),
          ...(accentColor ? { accentColor } : {}),
          continuation: { groupKey: continuationIdentity.continuationKey },
        });
        const withEventIdentity = (event: ProviderRuntimeEvent): ProviderRuntimeEvent => ({
          ...event,
          provider,
          providerInstanceId: instanceId,
        });
        const invoke = <A, E>(
          operation: string,
          effect: () => Effect.Effect<A, E>,
        ): Effect.Effect<A, ProviderAdapterRequestError> =>
          protectAdapterEffect(provider, operation, Effect.suspend(effect));
        const invalidOutput = (operation: string, error: unknown) =>
          adapterFailure(provider, `${operation}.decode`, error);
        const recoverList = (operation: string, cause: unknown) =>
          Effect.logError(`External provider ${operation} failed`, {
            provider,
            instanceId,
            cause: String(cause),
          }).pipe(Effect.andThen(Effect.succeed([] as ReadonlyArray<ProviderSession>)));
        const listSessions = () =>
          Effect.suspend(() => external.adapter.listSessions()).pipe(
            Effect.flatMap(decodeProviderSessions),
            Effect.map((sessions) => sessions.map(withSessionIdentity)),
            Effect.catch((error) => recoverList("listSessions", error)),
            Effect.catchDefect((defect) => recoverList("listSessions", defect)),
          );
        const hasSession = (threadId: ThreadId) =>
          Effect.suspend(() => external.adapter.hasSession(threadId)).pipe(
            Effect.flatMap(decodeBoolean),
            Effect.catch((error) =>
              Effect.logError("External provider hasSession failed", {
                provider,
                instanceId,
                cause: String(error),
              }).pipe(Effect.as(false)),
            ),
            Effect.catchDefect((defect) =>
              Effect.logError("External provider hasSession defected", {
                provider,
                instanceId,
                cause: String(defect),
              }).pipe(Effect.as(false)),
            ),
          );
        const recoverStream = <A>(operation: string, cause: Cause.Cause<unknown>) =>
          Cause.hasInterruptsOnly(cause)
            ? Stream.failCause(cause as Cause.Cause<never>)
            : Stream.fromEffect(
                Effect.logError(`External provider ${operation} stopped`, {
                  provider,
                  instanceId,
                  cause: String(cause),
                }),
              ).pipe(Stream.flatMap(() => Stream.empty as Stream.Stream<A>));
        const uploadFeedback = external.adapter.uploadFeedback;
        const adapter: ProviderInstance["adapter"] = {
          provider,
          adapterPackage: adapterPackageRef,
          capabilities: {
            sessionModelSwitch: capabilities.features.includes("model.switch")
              ? "in-session"
              : "unsupported",
            protocol: capabilities,
          },
          startSession: (input) =>
            invoke("startSession", () => external.adapter.startSession(input)).pipe(
              Effect.flatMap((session) =>
                decodeProviderSession(session).pipe(
                  Effect.mapError((error) => invalidOutput("startSession", error)),
                ),
              ),
              Effect.map(withSessionIdentity),
            ),
          sendTurn: (input) =>
            invoke("sendTurn", () => external.adapter.sendTurn(input)).pipe(
              Effect.flatMap((result) =>
                decodeTurnStartResult(result).pipe(
                  Effect.mapError((error) => invalidOutput("sendTurn", error)),
                ),
              ),
            ),
          interruptTurn: (threadId, turnId) =>
            invoke("interruptTurn", () => external.adapter.interruptTurn(threadId, turnId)),
          respondToRequest: (threadId, requestId, decision) =>
            invoke("respondToRequest", () =>
              external.adapter.respondToRequest(threadId, requestId, decision),
            ),
          respondToUserInput: (threadId, requestId, answers) =>
            invoke("respondToUserInput", () =>
              external.adapter.respondToUserInput(threadId, requestId, answers),
            ),
          stopSession: (threadId) =>
            invoke("stopSession", () => external.adapter.stopSession(threadId)),
          listSessions,
          hasSession,
          readThread: (threadId) =>
            invoke("readThread", () => external.adapter.readThread(threadId)).pipe(
              Effect.flatMap((snapshot) =>
                decodeThreadSnapshot(snapshot).pipe(
                  Effect.mapError((error) => invalidOutput("readThread", error)),
                ),
              ),
            ),
          rollbackThread: (threadId, numTurns) =>
            invoke("rollbackThread", () =>
              external.adapter.rollbackThread(threadId, numTurns),
            ).pipe(
              Effect.flatMap((snapshot) =>
                decodeThreadSnapshot(snapshot).pipe(
                  Effect.mapError((error) => invalidOutput("rollbackThread", error)),
                ),
              ),
            ),
          ...(uploadFeedback === undefined
            ? {}
            : {
                uploadFeedback: (input) => invoke("uploadFeedback", () => uploadFeedback(input)),
              }),
          stopAll: () => invoke("stopAll", () => external.adapter.stopAll()),
          streamEvents: Stream.merge(
            external.adapter.streamEvents.pipe(
              Stream.mapEffect((event) => decodeRuntimeEvent(event)),
              Stream.map(withEventIdentity),
              Stream.catchCause((cause) =>
                recoverStream<ProviderRuntimeEvent>("event stream", cause),
              ),
            ),
            currentHost.unexpectedExits.pipe(
              Stream.mapEffect((exit) =>
                DateTime.now.pipe(
                  Effect.map((now): ProviderRuntimeEvent => ({
                    eventId: EventId.make(NodeCrypto.randomUUID()),
                    provider,
                    providerInstanceId: instanceId,
                    threadId: exit.threadId,
                    createdAt: DateTime.formatIso(now),
                    type: "runtime.error",
                    payload: {
                      message: `Provider process ${exit.pid} exited unexpectedly with code ${exit.exitCode}.`,
                      class: "transport_error",
                      detail: { pid: exit.pid, exitCode: exit.exitCode },
                    },
                  })),
                ),
              ),
            ),
          ),
        };
        const unavailableSnapshot = (operation: string, cause: unknown) =>
          buildUnavailableProviderSnapshot({
            driverKind: provider,
            instanceId,
            displayName,
            accentColor,
            adapterPackage: adapterPackageRef,
            reason: `External adapter ${operation} failed: ${String(cause)}`,
          }).pipe(Effect.map(withSnapshotIdentity));
        const safeSnapshot = (operation: string, effect: Effect.Effect<ServerProvider>) =>
          Effect.suspend(() => effect).pipe(
            Effect.flatMap(decodeServerProvider),
            Effect.map(withSnapshotIdentity),
            Effect.catch((error) => unavailableSnapshot(operation, error)),
            Effect.catchDefect((defect) => unavailableSnapshot(operation, defect)),
          );
        // Adapters that declare a locally installed harness CLI opt into the
        // server's package-managed maintenance engine: update advisories on
        // their snapshots and a one-click update command derived from whoever
        // owns the executable. Adapters without the declaration stay
        // manual-only and publish snapshots untouched.
        const maintenance = adapterPackage.manifest.maintenance;
        const maintenanceServices = maintenance
          ? {
              httpClient: yield* HttpClient.HttpClient,
              serverSettings: yield* ServerSettingsService,
            }
          : null;
        const binaryPathFromConfig = (): string | null => {
          if (!maintenance) {
            return null;
          }
          const readKey = (value: unknown): string | null => {
            if (!isRecord(value)) {
              return null;
            }
            const raw = value[maintenance.binaryConfigKey];
            return typeof raw === "string" && raw.trim().length > 0 ? raw.trim() : null;
          };
          return readKey(config) ?? readKey(defaultConfig);
        };
        const resolveMaintenance: ProviderInstance["snapshot"]["resolveMaintenance"] = maintenance
          ? yield* makeCachedProviderMaintenanceResolution(
              resolveProviderMaintenanceCapabilitiesEffect(
                makePackageManagedProviderMaintenanceResolver({
                  provider,
                  npmPackageName: maintenance.npmPackage,
                  nativeUpdate: null,
                }),
                { binaryPath: binaryPathFromConfig(), env: processEnv },
              ).pipe(
                Effect.provideService(ChildProcessSpawner.ChildProcessSpawner, spawner),
                Effect.provideService(FileSystem.FileSystem, fileSystem),
                Effect.provideService(Path.Path, pathService),
              ),
            )
          : () =>
              Effect.succeed(
                makeManualOnlyProviderMaintenanceCapabilities({ provider, packageName: null }),
              );
        const enrichSnapshotWithMaintenance = (
          snapshot: ServerProvider,
        ): Effect.Effect<ServerProvider> =>
          maintenanceServices
            ? Effect.gen(function* () {
                const settings = yield* maintenanceServices.serverSettings.getSettings.pipe(
                  Effect.orElseSucceed(() => undefined),
                );
                const capabilities = yield* resolveMaintenance();
                const installedVersion =
                  typeof capabilities.installedVersion === "string"
                    ? capabilities.installedVersion.trim()
                    : null;
                // A maintained adapter's own version describes the adapter
                // package, not necessarily the harness CLI. Publish only a
                // version proven by the installer; unknown stays null rather
                // than fabricating an update comparison.
                const patched = {
                  ...snapshot,
                  version:
                    installedVersion !== null && installedVersion.length > 0
                      ? installedVersion
                      : null,
                };
                return yield* enrichProviderSnapshotWithVersionAdvisory(patched, capabilities, {
                  enableProviderUpdateChecks: settings?.enableProviderUpdateChecks,
                }).pipe(
                  Effect.provideService(HttpClient.HttpClient, maintenanceServices.httpClient),
                );
              }).pipe(
                // Enrichment must never take the snapshot channel down; an
                // un-enriched snapshot is stale but honest.
                Effect.catchCause((cause) =>
                  Effect.logWarning("External provider snapshot enrichment failed", {
                    provider,
                    instanceId,
                    cause: Cause.pretty(cause),
                  }).pipe(Effect.as(snapshot)),
                ),
              )
            : Effect.succeed(snapshot);
        const snapshot: ProviderInstance["snapshot"] = {
          resolveMaintenance,
          // V1 packages own their published snapshot, so a runtime usage-limit update cannot be
          // folded into it. External adapters that support limits report them inside snapshots.
          applyUsageLimits: () => Effect.void,
          getSnapshot: safeSnapshot("snapshot read", external.snapshot.getSnapshot).pipe(
            Effect.flatMap(enrichSnapshotWithMaintenance),
          ),
          refresh: safeSnapshot("snapshot refresh", external.snapshot.refresh).pipe(
            Effect.flatMap(enrichSnapshotWithMaintenance),
          ),
          streamChanges: external.snapshot.streamChanges.pipe(
            Stream.mapEffect((snapshot) => decodeServerProvider(snapshot)),
            Stream.map(withSnapshotIdentity),
            Stream.mapEffect(enrichSnapshotWithMaintenance),
            Stream.catchCause((cause) =>
              Cause.hasInterruptsOnly(cause)
                ? Stream.failCause(cause as Cause.Cause<never>)
                : Stream.fromEffect(unavailableSnapshot("snapshot stream", cause)),
            ),
          ),
        };

        return {
          instanceId,
          driverKind: provider,
          adapterPackage: adapterPackageRef,
          continuationIdentity,
          displayName,
          accentColor,
          enabled,
          snapshot,
          adapter,
          textGeneration: makeUnsupportedTextGeneration(provider),
        } satisfies ProviderInstance;
      }),
  };
};
