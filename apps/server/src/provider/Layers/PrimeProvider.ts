// @effect-diagnostics nodeBuiltinImport:off
import * as NodeFS from "node:fs";
import * as NodePath from "node:path";

import { type PrimeSettings, type ServerProviderModel } from "@t3tools/contracts";
import * as DateTime from "effect/DateTime";
import * as Effect from "effect/Effect";
import * as FileSystem from "effect/FileSystem";
import * as Option from "effect/Option";
import * as Path from "effect/Path";
import * as Result from "effect/Result";
import * as Schema from "effect/Schema";
import { ChildProcess, ChildProcessSpawner } from "effect/unstable/process";
import { resolveSpawnCommand } from "@t3tools/shared/shell";

import {
  buildServerProvider,
  isCommandMissingCause,
  parseGenericCliVersion,
  spawnAndCollect,
  type ServerProviderDraft,
} from "../providerSnapshot.ts";
import { PrimeRpcError, spawnPrimeRpcClient } from "../prime/PrimeRpcClient.ts";
import {
  isPrimeApprovalExtensionHandshake,
  preparePrimeApprovalExtension,
  PRIME_APPROVAL_EXTENSION_MODE_FLAG,
} from "../prime/primeApprovalExtension.ts";
import {
  loggedInPrimeProvidersFromAuthData,
  mapPrimeAvailableModels,
} from "../prime/primeModels.ts";
import {
  primePackageCatalogExtensionArgs,
  resolvePrimeAgentDir,
} from "../prime/primePackageCatalogExtensions.ts";
import { preparePrimeOpenRouterCatalogExtension } from "../prime/primeOpenRouterCatalogExtension.ts";

const PRIME_PRESENTATION = {
  displayName: "Prime Agent",
  badgeLabel: "Early Access",
  showInteractionModeToggle: false,
  requiresNewThreadForModelChange: false,
} as const;

const VERSION_PROBE_TIMEOUT_MS = 4_000;
const MODELS_PROBE_TIMEOUT_MS = 10_000;
const APPROVAL_PROBE_TIMEOUT_MS = 4_000;
const EMPTY_MODELS = [] as const;

const buildPrimeServerProvider = (
  input: Parameters<typeof buildServerProvider>[0],
  approvalRequired = false,
): ServerProviderDraft => ({
  ...buildServerProvider(input),
  supportedRuntimeModes: approvalRequired ? ["full-access", "approval-required"] : ["full-access"],
});

const PrimeListedModel = Schema.Struct({
  id: Schema.String,
  name: Schema.optional(Schema.String),
  provider: Schema.optional(Schema.String),
  thinkingLevelMap: Schema.optional(Schema.Record(Schema.String, Schema.NullOr(Schema.String))),
});

const PrimeAvailableModels = Schema.Struct({
  models: Schema.Array(PrimeListedModel),
});

const decodePrimeAvailableModels = Schema.decodeUnknownEffect(PrimeAvailableModels);
const decodePrimeAuthFile = Schema.decodeUnknownSync(
  Schema.fromJsonString(Schema.Record(Schema.String, Schema.Unknown)),
);

function readLoggedInPrimeProviders(
  environment: NodeJS.ProcessEnv,
): ReadonlySet<string> | undefined {
  const authPath = NodePath.join(resolvePrimeAgentDir(environment), "auth.json");
  try {
    if (!NodeFS.existsSync(authPath)) {
      return undefined;
    }
    const raw = NodeFS.readFileSync(authPath, "utf8");
    return loggedInPrimeProvidersFromAuthData(decodePrimeAuthFile(raw));
  } catch {
    return undefined;
  }
}

const runPrimeVersionCommand = (
  primeSettings: PrimeSettings,
  environment: NodeJS.ProcessEnv = process.env,
) =>
  Effect.gen(function* () {
    const command = primeSettings.binaryPath || "prime-agent";
    const spawnCommand = yield* resolveSpawnCommand(command, ["--version"], {
      env: environment,
    });
    return yield* spawnAndCollect(
      command,
      ChildProcess.make(spawnCommand.command, spawnCommand.args, {
        env: environment,
        shell: spawnCommand.shell,
      }),
    );
  });

const listPrimeModels = (
  primeSettings: PrimeSettings,
  environment: NodeJS.ProcessEnv,
  extensionBaseDir?: string,
): Effect.Effect<
  ReadonlyArray<ServerProviderModel>,
  PrimeRpcError,
  ChildProcessSpawner.ChildProcessSpawner | FileSystem.FileSystem | Path.Path
> =>
  Effect.scoped(
    Effect.gen(function* () {
      const catalogExtensionPath = extensionBaseDir
        ? yield* preparePrimeOpenRouterCatalogExtension(extensionBaseDir).pipe(
            Effect.mapError(
              (cause) =>
                new PrimeRpcError({
                  operation: "get_available_models",
                  detail: "Could not prepare the OpenRouter catalog extension.",
                  cause,
                }),
            ),
          )
        : undefined;
      const packageCatalogExtensionArgs = primePackageCatalogExtensionArgs(
        resolvePrimeAgentDir(environment),
      );
      const rpc = yield* spawnPrimeRpcClient({
        command: primeSettings.binaryPath || "prime-agent",
        args: [
          "--mode",
          "rpc",
          "--no-session",
          "--no-tools",
          "--no-extensions",
          ...(catalogExtensionPath ? ["--extension", catalogExtensionPath] : []),
          ...packageCatalogExtensionArgs,
        ],
        cwd: process.cwd(),
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

export interface PrimeProviderStatusOptions {
  readonly approvalExtensionBaseDir?: string;
}

const probePrimeApprovalExtension = (
  primeSettings: PrimeSettings,
  environment: NodeJS.ProcessEnv,
  baseDir: string,
) =>
  Effect.scoped(
    Effect.gen(function* () {
      const extensionPath = yield* preparePrimeApprovalExtension(baseDir);
      const rpc = yield* spawnPrimeRpcClient({
        command: primeSettings.binaryPath || "prime-agent",
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
        cwd: process.cwd(),
        environment,
      });
      const response = yield* rpc.request(
        { type: "get_commands" },
        { timeoutMs: APPROVAL_PROBE_TIMEOUT_MS },
      );
      return isPrimeApprovalExtensionHandshake(response.data, extensionPath);
    }),
  );

export function buildInitialPrimeProviderSnapshot(
  primeSettings: PrimeSettings,
): Effect.Effect<ServerProviderDraft> {
  return Effect.gen(function* () {
    const checkedAt = yield* Effect.map(DateTime.now, DateTime.formatIso);

    if (!primeSettings.enabled) {
      return buildPrimeServerProvider({
        presentation: PRIME_PRESENTATION,
        enabled: false,
        checkedAt,
        models: EMPTY_MODELS,
        probe: {
          installed: false,
          version: null,
          status: "warning",
          auth: { status: "unknown" },
          message: "Prime Agent is disabled in T3 Code settings.",
        },
      });
    }

    return buildPrimeServerProvider({
      presentation: PRIME_PRESENTATION,
      enabled: true,
      checkedAt,
      models: EMPTY_MODELS,
      probe: {
        installed: true,
        version: null,
        status: "warning",
        auth: { status: "unknown" },
        message: "Checking Prime Agent CLI availability...",
      },
    });
  });
}

export const checkPrimeProviderStatus = Effect.fn("checkPrimeProviderStatus")(function* (
  primeSettings: PrimeSettings,
  environment: NodeJS.ProcessEnv = process.env,
  options: PrimeProviderStatusOptions = {},
): Effect.fn.Return<
  ServerProviderDraft,
  never,
  ChildProcessSpawner.ChildProcessSpawner | FileSystem.FileSystem | Path.Path
> {
  const checkedAt = DateTime.formatIso(yield* DateTime.now);

  if (!primeSettings.enabled) {
    return buildPrimeServerProvider({
      presentation: PRIME_PRESENTATION,
      enabled: false,
      checkedAt,
      models: EMPTY_MODELS,
      probe: {
        installed: false,
        version: null,
        status: "warning",
        auth: { status: "unknown" },
        message: "Prime Agent is disabled in T3 Code settings.",
      },
    });
  }

  const versionResult = yield* runPrimeVersionCommand(primeSettings, environment).pipe(
    Effect.timeoutOption(VERSION_PROBE_TIMEOUT_MS),
    Effect.result,
  );

  if (Result.isFailure(versionResult)) {
    const error = versionResult.failure;
    yield* Effect.logWarning("Prime Agent CLI health check failed.", {
      errorTag: error._tag,
    });
    return buildPrimeServerProvider({
      presentation: PRIME_PRESENTATION,
      enabled: primeSettings.enabled,
      checkedAt,
      models: EMPTY_MODELS,
      probe: {
        installed: !isCommandMissingCause(error),
        version: null,
        status: "error",
        auth: { status: "unknown" },
        message: isCommandMissingCause(error)
          ? "Prime Agent (`prime-agent`) is not installed or not on PATH."
          : "Failed to execute Prime Agent CLI health check.",
      },
    });
  }

  if (Option.isNone(versionResult.success)) {
    return buildPrimeServerProvider({
      presentation: PRIME_PRESENTATION,
      enabled: primeSettings.enabled,
      checkedAt,
      models: EMPTY_MODELS,
      probe: {
        installed: true,
        version: null,
        status: "error",
        auth: { status: "unknown" },
        message: "Prime Agent is installed but timed out while running `prime-agent --version`.",
      },
    });
  }

  const versionOutput = versionResult.success.value;
  const version = parseGenericCliVersion(`${versionOutput.stdout}\n${versionOutput.stderr}`);
  if (versionOutput.code !== 0) {
    yield* Effect.logWarning("Prime Agent version probe exited with a non-zero status.", {
      exitCode: versionOutput.code,
      stdoutLength: versionOutput.stdout.length,
      stderrLength: versionOutput.stderr.length,
    });
    return buildPrimeServerProvider({
      presentation: PRIME_PRESENTATION,
      enabled: primeSettings.enabled,
      checkedAt,
      models: EMPTY_MODELS,
      probe: {
        installed: true,
        version,
        status: "error",
        auth: { status: "unknown" },
        message: "Prime Agent is installed but failed to run.",
      },
    });
  }

  const approvalResult = options.approvalExtensionBaseDir
    ? yield* probePrimeApprovalExtension(
        primeSettings,
        environment,
        options.approvalExtensionBaseDir,
      ).pipe(Effect.timeoutOption(APPROVAL_PROBE_TIMEOUT_MS), Effect.result)
    : undefined;
  const approvalRequired =
    approvalResult !== undefined &&
    Result.isSuccess(approvalResult) &&
    Option.isSome(approvalResult.success) &&
    approvalResult.success.value;
  const presentation = PRIME_PRESENTATION;

  const modelsResult = yield* listPrimeModels(
    primeSettings,
    environment,
    options.approvalExtensionBaseDir,
  ).pipe(Effect.timeoutOption(MODELS_PROBE_TIMEOUT_MS), Effect.result);

  if (Result.isFailure(modelsResult) || Option.isNone(modelsResult.success)) {
    yield* Effect.logWarning("Prime Agent could not list models.", {
      timedOut: Result.isSuccess(modelsResult) && Option.isNone(modelsResult.success),
    });
    return buildPrimeServerProvider(
      {
        presentation,
        enabled: primeSettings.enabled,
        checkedAt,
        models: EMPTY_MODELS,
        probe: {
          installed: true,
          version,
          status: "warning",
          auth: { status: "unknown" },
          message:
            Result.isSuccess(modelsResult) && Option.isNone(modelsResult.success)
              ? "Prime Agent is installed but timed out while listing models."
              : "Prime Agent is installed but could not list models.",
        },
      },
      approvalRequired,
    );
  }

  const models = modelsResult.success.value;
  if (models.length === 0) {
    return buildPrimeServerProvider(
      {
        presentation,
        enabled: primeSettings.enabled,
        checkedAt,
        models: EMPTY_MODELS,
        probe: {
          installed: true,
          version,
          status: "warning",
          auth: { status: "unauthenticated" },
          message: "Prime Agent is installed but has no logged-in providers.",
        },
      },
      approvalRequired,
    );
  }

  return buildPrimeServerProvider(
    {
      presentation,
      enabled: primeSettings.enabled,
      checkedAt,
      models,
      probe: {
        installed: true,
        version,
        status: "ready",
        auth: { status: "authenticated" },
      },
    },
    approvalRequired,
  );
});
