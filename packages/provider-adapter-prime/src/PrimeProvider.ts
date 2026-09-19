// @effect-diagnostics nodeBuiltinImport:off
import * as NodeFS from "node:fs";
import * as NodeOS from "node:os";
import * as NodePath from "node:path";

import type {
  ProviderAdapterCreateInputV1,
  ProviderAdapterHostV2,
  ProviderAdapterSnapshotV1,
} from "@t3tools/provider-adapter";
import {
  type ProviderAdapterPackageReference,
  type ServerProvider,
  type ServerProviderModel,
  ProviderDriverKind,
} from "@t3tools/contracts";
import * as DateTime from "effect/DateTime";
import * as Effect from "effect/Effect";
import * as Option from "effect/Option";
import * as PlatformError from "effect/PlatformError";
import * as PubSub from "effect/PubSub";
import * as Result from "effect/Result";
import * as Schema from "effect/Schema";
import type * as Scope from "effect/Scope";
import * as Semaphore from "effect/Semaphore";
import * as Stream from "effect/Stream";

import type { PrimeAdapterConfig } from "./PrimeAdapter.ts";
import { makePrimeRpcClient, PrimeRpcError } from "./PrimeRpcClient.ts";
import {
  isPrimeApprovalExtensionHandshake,
  preparePrimeApprovalExtension,
  PRIME_APPROVAL_EXTENSION_MODE_FLAG,
} from "./primeApprovalExtension.ts";
import { loggedInPrimeProvidersFromAuthData, mapPrimeAvailableModels } from "./primeModels.ts";
import { primePackageCatalogExtensionArgs } from "./primePackageCatalogExtensions.ts";
import { preparePrimeOpenRouterCatalogExtension } from "./primeOpenRouterCatalogExtension.ts";

export const PRIME_PRESENTATION = {
  displayName: "Prime Agent",
  badgeLabel: "Early Access",
  showInteractionModeToggle: false,
  requiresNewThreadForModelChange: false,
} as const;

const VERSION_PROBE_TIMEOUT_MS = 4_000;
const MODELS_PROBE_TIMEOUT_MS = 10_000;
const APPROVAL_PROBE_TIMEOUT_MS = 4_000;

const PrimeListedModel = Schema.Struct({
  id: Schema.String,
  name: Schema.optional(Schema.String),
  provider: Schema.optional(Schema.String),
  thinkingLevelMap: Schema.optional(Schema.Record(Schema.String, Schema.NullOr(Schema.String))),
});
const PrimeAvailableModels = Schema.Struct({ models: Schema.Array(PrimeListedModel) });
const decodePrimeAvailableModels = Schema.decodeUnknownEffect(PrimeAvailableModels);
const decodePrimeAuthFile = Schema.decodeUnknownSync(
  Schema.fromJsonString(Schema.Record(Schema.String, Schema.Unknown)),
);

const packageReference = (input: PrimeSnapshotOptions): ProviderAdapterPackageReference => ({
  id: input.packageId,
  version: input.packageVersion,
  protocolVersion: 1,
});

const resolvePrimeAgentDir = (
  environment: Readonly<Record<string, string | undefined>>,
): string => {
  const configured = environment.PRIME_AGENT_CODING_AGENT_DIR?.trim();
  if (!configured) return NodePath.join(NodeOS.homedir(), ".prime", "agent");
  if (configured === "~") return NodeOS.homedir();
  if (configured.startsWith("~/") || configured.startsWith("~\\")) {
    return NodePath.join(NodeOS.homedir(), configured.slice(2));
  }
  return configured;
};

const readLoggedInPrimeProviders = (
  environment: Readonly<Record<string, string | undefined>>,
): ReadonlySet<string> | undefined => {
  const authPath = NodePath.join(resolvePrimeAgentDir(environment), "auth.json");
  try {
    if (!NodeFS.existsSync(authPath)) return undefined;
    return loggedInPrimeProvidersFromAuthData(
      decodePrimeAuthFile(NodeFS.readFileSync(authPath, "utf8")),
    );
  } catch {
    return undefined;
  }
};

const parseGenericCliVersion = (output: string): string | null =>
  output.match(/\b(\d+\.\d+\.\d+)\b/)?.[1] ?? null;

const collectText = <E>(stream: Stream.Stream<Uint8Array, E>) =>
  stream.pipe(
    Stream.decodeText(),
    Stream.runFold(
      () => "",
      (all, chunk) => all + chunk,
    ),
  );

const spawnAndCollect = (
  host: ProviderAdapterHostV2,
  input: {
    readonly command: string;
    readonly args: ReadonlyArray<string>;
    readonly cwd: string;
    readonly environment: Readonly<Record<string, string | undefined>>;
  },
) =>
  Effect.scoped(
    Effect.gen(function* () {
      const process = yield* host.processes.spawn({ ...input, purpose: { kind: "probe" } });
      yield* process.expectExit;
      const [stdout, stderr, code] = yield* Effect.all(
        [collectText(process.stdout), collectText(process.stderr), process.exitCode],
        { concurrency: "unbounded" },
      );
      return { stdout, stderr, code };
    }),
  );

const makePrimeProbeRpcClient = (
  host: ProviderAdapterHostV2,
  input: {
    readonly command: string;
    readonly args: ReadonlyArray<string>;
    readonly cwd: string;
    readonly environment: Readonly<Record<string, string | undefined>>;
  },
) =>
  Effect.gen(function* () {
    const process = yield* host.processes
      .spawn({ ...input, purpose: { kind: "probe" } })
      .pipe(
        Effect.mapError(
          (cause) => new PrimeRpcError({ operation: "spawn", detail: cause.detail, cause }),
        ),
      );
    yield* process.expectExit;
    return yield* makePrimeRpcClient(process);
  });

const listPrimeModels = (
  config: PrimeAdapterConfig,
  environment: Readonly<Record<string, string | undefined>>,
  cwd: string,
  host: ProviderAdapterHostV2,
): Effect.Effect<ReadonlyArray<ServerProviderModel>, PrimeRpcError> =>
  Effect.scoped(
    Effect.gen(function* () {
      const catalogExtensionPath = yield* preparePrimeOpenRouterCatalogExtension(host.storage).pipe(
        Effect.mapError(
          (cause) =>
            new PrimeRpcError({
              operation: "get_available_models",
              detail: "Could not prepare the OpenRouter catalog extension.",
              cause,
            }),
        ),
      );
      const rpc = yield* makePrimeProbeRpcClient(host, {
        command: config.binaryPath || "prime-agent",
        args: [
          "--mode",
          "rpc",
          "--no-session",
          "--no-tools",
          "--no-extensions",
          "--extension",
          catalogExtensionPath,
          ...primePackageCatalogExtensionArgs(resolvePrimeAgentDir(environment)),
        ],
        cwd,
        environment,
      });
      const response = yield* rpc.request(
        { type: "get_available_models" },
        { timeoutMs: MODELS_PROBE_TIMEOUT_MS },
      );
      const decoded = yield* decodePrimeAvailableModels(response.data).pipe(
        Effect.mapError(
          (cause) =>
            new PrimeRpcError({
              operation: "get_available_models",
              detail: "Prime returned an invalid model list.",
              cause,
            }),
        ),
      );
      const loggedInProviders = readLoggedInPrimeProviders(environment);
      return mapPrimeAvailableModels(
        decoded.models,
        loggedInProviders === undefined ? undefined : { loggedInProviders },
      );
    }),
  );

const probePrimeApprovalExtension = (
  config: PrimeAdapterConfig,
  environment: Readonly<Record<string, string | undefined>>,
  cwd: string,
  host: ProviderAdapterHostV2,
) =>
  Effect.scoped(
    Effect.gen(function* () {
      const extensionPath = yield* preparePrimeApprovalExtension(host.storage).pipe(
        Effect.mapError(
          (cause) =>
            new PrimeRpcError({ operation: "approval_probe", detail: cause.detail, cause }),
        ),
      );
      const rpc = yield* makePrimeProbeRpcClient(host, {
        command: config.binaryPath || "prime-agent",
        args: [
          "--mode",
          "rpc",
          "--no-session",
          "--no-tools",
          "--no-extensions",
          "--extension",
          extensionPath,
          `--${PRIME_APPROVAL_EXTENSION_MODE_FLAG}=approval-required`,
        ],
        cwd,
        environment,
      });
      const response = yield* rpc.request(
        { type: "get_commands" },
        { timeoutMs: APPROVAL_PROBE_TIMEOUT_MS },
      );
      return isPrimeApprovalExtensionHandshake(response.data, extensionPath);
    }),
  );

export interface PrimeSnapshotOptions {
  readonly packageId: ProviderAdapterPackageReference["id"];
  readonly packageVersion: ProviderAdapterPackageReference["version"];
  readonly manifestConfigSchema: ServerProvider["adapterConfigSchema"];
  readonly capabilities: NonNullable<ServerProvider["adapterCapabilities"]>;
}

const baseSnapshot = (
  input: ProviderAdapterCreateInputV1<PrimeAdapterConfig>,
  options: PrimeSnapshotOptions,
  checkedAt: string,
): ServerProvider => ({
  instanceId: input.instanceId,
  driver: ProviderDriverKind.make("primeAgent"),
  adapterPackage: packageReference(options),
  adapterConfigSchema: options.manifestConfigSchema,
  adapterCapabilities: options.capabilities,
  displayName: input.displayName ?? PRIME_PRESENTATION.displayName,
  ...(input.accentColor ? { accentColor: input.accentColor } : {}),
  badgeLabel: PRIME_PRESENTATION.badgeLabel,
  showInteractionModeToggle: PRIME_PRESENTATION.showInteractionModeToggle,
  requiresNewThreadForModelChange: PRIME_PRESENTATION.requiresNewThreadForModelChange,
  supportedRuntimeModes: ["full-access"],
  enabled: input.enabled,
  installed: input.enabled,
  version: null,
  status: input.enabled ? "warning" : "disabled",
  auth: { status: "unknown" },
  checkedAt,
  message: input.enabled
    ? "Checking Prime Agent CLI availability..."
    : "Prime Agent is disabled in T3 Code settings.",
  models: [],
  slashCommands: [],
  skills: [],
});

const commandAppearsMissing = (cause: unknown, seen = new Set<unknown>()): boolean => {
  if (seen.has(cause)) return false;
  seen.add(cause);
  if (cause instanceof PlatformError.PlatformError && cause.reason._tag === "NotFound") {
    return true;
  }
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
};

export const checkPrimeProviderStatus = Effect.fn("checkPrimeProviderStatus")(function* (
  input: ProviderAdapterCreateInputV1<PrimeAdapterConfig>,
  host: ProviderAdapterHostV2,
  options: PrimeSnapshotOptions,
): Effect.fn.Return<ServerProvider> {
  const checkedAt = DateTime.formatIso(yield* DateTime.now);
  const initial = baseSnapshot(input, options, checkedAt);
  if (!input.enabled) return initial;

  const cwdResult = yield* host.workspaces.resolveCwd().pipe(Effect.result);
  if (Result.isFailure(cwdResult)) {
    return {
      ...initial,
      installed: true,
      status: "error" as const,
      message: cwdResult.failure.detail,
    };
  }
  const cwd = cwdResult.success;
  const versionResult = yield* spawnAndCollect(host, {
    command: input.config.binaryPath || "prime-agent",
    args: ["--version"],
    cwd,
    environment: input.environment,
  }).pipe(Effect.timeoutOption(VERSION_PROBE_TIMEOUT_MS), Effect.result);

  if (Result.isFailure(versionResult)) {
    const missing = commandAppearsMissing(versionResult.failure);
    return {
      ...initial,
      installed: !missing,
      status: "error" as const,
      message: missing
        ? "Prime Agent (`prime-agent`) is not installed or not on PATH."
        : "Failed to execute Prime Agent CLI health check.",
    };
  }
  if (Option.isNone(versionResult.success)) {
    return {
      ...initial,
      installed: true,
      status: "error" as const,
      message: "Prime Agent is installed but timed out while running `prime-agent --version`.",
    };
  }
  const versionOutput = versionResult.success.value;
  const version = parseGenericCliVersion(`${versionOutput.stdout}\n${versionOutput.stderr}`);
  if (versionOutput.code !== 0) {
    return {
      ...initial,
      installed: true,
      version,
      status: "error" as const,
      message: "Prime Agent is installed but failed to run.",
    };
  }

  const approvalResult = yield* probePrimeApprovalExtension(
    input.config,
    input.environment,
    cwd,
    host,
  ).pipe(Effect.timeoutOption(APPROVAL_PROBE_TIMEOUT_MS), Effect.result);
  const approvalRequired =
    Result.isSuccess(approvalResult) &&
    Option.isSome(approvalResult.success) &&
    approvalResult.success.value;
  const supportedRuntimeModes: ServerProvider["supportedRuntimeModes"] = approvalRequired
    ? ["full-access", "approval-required"]
    : ["full-access"];

  const modelsResult = yield* listPrimeModels(input.config, input.environment, cwd, host).pipe(
    Effect.timeoutOption(MODELS_PROBE_TIMEOUT_MS),
    Effect.result,
  );
  if (Result.isFailure(modelsResult) || Option.isNone(modelsResult.success)) {
    return {
      ...initial,
      installed: true,
      version,
      supportedRuntimeModes,
      status: "warning" as const,
      message:
        Result.isSuccess(modelsResult) && Option.isNone(modelsResult.success)
          ? "Prime Agent is installed but timed out while listing models."
          : "Prime Agent is installed but could not list models.",
    };
  }
  const models = modelsResult.success.value;
  if (models.length === 0) {
    return {
      ...initial,
      installed: true,
      version,
      supportedRuntimeModes,
      status: "warning" as const,
      auth: { status: "unauthenticated" },
      message: "Prime Agent is installed but has no logged-in providers.",
    };
  }
  const { message: _message, ...withoutMessage } = initial;
  return {
    ...withoutMessage,
    installed: true,
    version,
    supportedRuntimeModes,
    status: "ready" as const,
    auth: { status: "authenticated" },
    models,
  };
});

export const makePrimeProviderSnapshot = Effect.fn("makePrimeProviderSnapshot")(function* (
  input: ProviderAdapterCreateInputV1<PrimeAdapterConfig>,
  host: ProviderAdapterHostV2,
  options: PrimeSnapshotOptions,
): Effect.fn.Return<ProviderAdapterSnapshotV1, never, Scope.Scope> {
  const changes = yield* Effect.acquireRelease(PubSub.unbounded<ServerProvider>(), PubSub.shutdown);
  const refreshSemaphore = yield* Semaphore.make(1);
  let current = baseSnapshot(input, options, DateTime.formatIso(yield* DateTime.now));
  const refresh = refreshSemaphore.withPermits(1)(
    checkPrimeProviderStatus(input, host, options).pipe(
      Effect.tap((snapshot) =>
        Effect.sync(() => {
          current = snapshot;
        }).pipe(Effect.andThen(PubSub.publish(changes, snapshot))),
      ),
    ),
  );
  // Match built-in provider startup behavior: publish the pending snapshot
  // immediately, then probe without blocking registry construction.
  yield* refresh.pipe(Effect.ignoreCause({ log: true }), Effect.forkScoped);
  return {
    getSnapshot: Effect.sync(() => current),
    refresh,
    streamChanges: Stream.fromPubSub(changes),
  };
});
