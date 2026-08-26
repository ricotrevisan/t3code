// @effect-diagnostics nodeBuiltinImport:off globalConsole:off -- direnv export is a subprocess boundary; a broken .envrc is surfaced as a server warning.
/**
 * workspaceDirenvEnv - Load a workspace directory's direnv-exported env.
 *
 * T3 spawns project-scoped children (terminals, Codex, Prime Agent, ACP,
 * OpenCode, Claude, ProcessRunner) with the daemon environment rather than an
 * interactive shell, so `.envrc` / direnv / varlock never run. This helper
 * asks direnv for the export overlay of `cwd` and returns it for callers to
 * merge into spawn `env`.
 *
 * Fail-soft: missing `.envrc`, missing direnv, not-allowed, or export failure
 * yields an empty overlay and a warning that never includes secret values.
 * Results are cached per resolved directory so a cwd change reloads while
 * repeated spawns in the same workspace do not re-run varlock.
 *
 * @module workspaceDirenvEnv
 */
import * as NodeChildProcess from "node:child_process";
import * as NodeFS from "node:fs";
import * as NodePath from "node:path";

const DIRENV_EXPORT_TIMEOUT_MS = 20_000;
const DIRENV_EXPORT_MAX_BUFFER_BYTES = 2 * 1024 * 1024;
const DIRENV_INTERNAL_PREFIX = "DIRENV_";

export interface DirenvExportRunResult {
  readonly status: number | null;
  readonly stdout: string;
  readonly stderr: string;
  readonly error?: NodeJS.ErrnoException | undefined;
}

export interface LoadDirenvExportedEnvOptions {
  readonly env?: NodeJS.ProcessEnv | undefined;
  readonly runExport?: (input: {
    readonly direnvPath: string;
    readonly cwd: string;
    readonly env: NodeJS.ProcessEnv;
  }) => DirenvExportRunResult;
  readonly resolveDirenvPath?: (env: NodeJS.ProcessEnv) => string | undefined;
  readonly findEnvrc?: (cwd: string) => string | undefined;
  readonly logger?: (message: string) => void;
}

const overlayCache = new Map<string, NodeJS.ProcessEnv>();
const warnedCacheKeys = new Set<string>();

export function resetWorkspaceDirenvEnvCache(): void {
  overlayCache.clear();
  warnedCacheKeys.clear();
}

function defaultLogger(message: string): void {
  console.warn(message);
}

function warnOnce(cacheKey: string, message: string, logger: (message: string) => void): void {
  if (warnedCacheKeys.has(cacheKey)) return;
  warnedCacheKeys.add(cacheKey);
  logger(message);
}

function envToSpawnEnv(env: NodeJS.ProcessEnv): NodeJS.ProcessEnv {
  const spawnEnv: NodeJS.ProcessEnv = {};
  for (const [key, value] of Object.entries(env)) {
    if (value === undefined) continue;
    spawnEnv[key] = value;
  }
  return spawnEnv;
}

export function findEnvrcPath(cwd: string): string | undefined {
  let dir = NodePath.resolve(cwd);
  for (;;) {
    const candidate = NodePath.join(dir, ".envrc");
    if (NodeFS.existsSync(candidate)) return candidate;
    const parent = NodePath.dirname(dir);
    if (parent === dir) return undefined;
    dir = parent;
  }
}

export function resolveDirenvPath(env: NodeJS.ProcessEnv): string | undefined {
  const configured = env.DIRENV_BIN?.trim();
  if (configured && NodeFS.existsSync(configured)) return configured;

  const executable = process.platform === "win32" ? "direnv.exe" : "direnv";
  const pathValue = env.PATH ?? "";
  for (const dir of pathValue.split(NodePath.delimiter)) {
    if (dir.length === 0) continue;
    const candidate = NodePath.join(dir, executable);
    if (NodeFS.existsSync(candidate)) return candidate;
  }

  const fallback = process.platform === "win32" ? undefined : "/usr/bin/direnv";
  if (fallback && NodeFS.existsSync(fallback)) return fallback;
  return undefined;
}

function defaultRunExport(input: {
  readonly direnvPath: string;
  readonly cwd: string;
  readonly env: NodeJS.ProcessEnv;
}): DirenvExportRunResult {
  const result = NodeChildProcess.spawnSync(input.direnvPath, ["export", "json"], {
    cwd: input.cwd,
    env: envToSpawnEnv(input.env),
    encoding: "utf8",
    timeout: DIRENV_EXPORT_TIMEOUT_MS,
    maxBuffer: DIRENV_EXPORT_MAX_BUFFER_BYTES,
    windowsHide: true,
  });
  const error = result.error as NodeJS.ErrnoException | undefined;
  return {
    status: result.status,
    stdout: typeof result.stdout === "string" ? result.stdout : "",
    stderr: typeof result.stderr === "string" ? result.stderr : "",
    ...(error ? { error } : {}),
  };
}

function parseDirenvExportJson(stdout: string): NodeJS.ProcessEnv | undefined {
  const trimmed = stdout.trim();
  if (trimmed.length === 0) return {};
  try {
    const parsed: unknown = JSON.parse(trimmed);
    if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
      return undefined;
    }
    const overlay: NodeJS.ProcessEnv = {};
    for (const [key, value] of Object.entries(parsed as Record<string, unknown>)) {
      if (key.length === 0 || key.startsWith(DIRENV_INTERNAL_PREFIX)) continue;
      if (typeof value !== "string") continue;
      overlay[key] = value;
    }
    return overlay;
  } catch {
    return undefined;
  }
}

function stripAnsi(value: string): string {
  return value.replace(/\u001b\[[0-9;]*m/g, "").trim();
}

function classifyExportFailure(result: DirenvExportRunResult): string {
  const stderr = stripAnsi(result.stderr);
  if (result.error?.code === "ENOENT") return "direnv missing";
  if (result.error?.code === "ETIMEDOUT") return "direnv export timed out";
  if (/is blocked|direnv allow/i.test(stderr)) return "direnv allow not done";
  if (/command not found/i.test(stderr)) return "direnv export failed";
  if (result.error) return "direnv export failed";
  return "direnv export failed";
}

function exportLooksFailed(result: DirenvExportRunResult): boolean {
  if (result.status !== 0 || result.error) return true;
  const stderr = stripAnsi(result.stderr);
  return /is blocked|direnv allow|command not found/i.test(stderr);
}

/**
 * Return the direnv export overlay for `cwd` (exported keys only, no
 * `DIRENV_*` internals). Empty when there is nothing to load.
 */
export function loadDirenvExportedEnv(
  cwd: string | undefined,
  options: LoadDirenvExportedEnvOptions = {},
): NodeJS.ProcessEnv {
  if (cwd === undefined || cwd.trim().length === 0) return {};

  const loaderEnv = options.env ?? process.env;
  const logger = options.logger ?? defaultLogger;
  let resolvedCwd: string;
  try {
    resolvedCwd = NodePath.resolve(cwd);
  } catch {
    return {};
  }

  const cached = overlayCache.get(resolvedCwd);
  if (cached !== undefined) return cached;

  const findEnvrc = options.findEnvrc ?? findEnvrcPath;
  const envrcPath = findEnvrc(resolvedCwd);
  if (envrcPath === undefined) {
    overlayCache.set(resolvedCwd, {});
    return {};
  }

  const resolveDirenv = options.resolveDirenvPath ?? resolveDirenvPath;
  const direnvPath = resolveDirenv(loaderEnv);
  if (direnvPath === undefined) {
    warnOnce(
      resolvedCwd,
      `direnv: skipped workspace env for '${resolvedCwd}' (direnv missing)`,
      logger,
    );
    overlayCache.set(resolvedCwd, {});
    return {};
  }

  const runExport = options.runExport ?? defaultRunExport;
  let result: DirenvExportRunResult;
  try {
    result = runExport({ direnvPath, cwd: resolvedCwd, env: loaderEnv });
  } catch {
    warnOnce(
      resolvedCwd,
      `direnv: skipped workspace env for '${resolvedCwd}' (direnv export failed)`,
      logger,
    );
    overlayCache.set(resolvedCwd, {});
    return {};
  }

  if (exportLooksFailed(result)) {
    warnOnce(
      resolvedCwd,
      `direnv: skipped workspace env for '${resolvedCwd}' (${classifyExportFailure(result)})`,
      logger,
    );
    overlayCache.set(resolvedCwd, {});
    return {};
  }

  const overlay = parseDirenvExportJson(result.stdout);
  if (overlay === undefined) {
    warnOnce(
      resolvedCwd,
      `direnv: skipped workspace env for '${resolvedCwd}' (direnv export failed)`,
      logger,
    );
    overlayCache.set(resolvedCwd, {});
    return {};
  }

  overlayCache.set(resolvedCwd, overlay);
  return overlay;
}

/**
 * Merge a directory's direnv overlay onto `env`. `env` wins for keys that
 * direnv does not export; direnv wins on conflict so children match
 * interactive `cd`.
 */
export function mergeDirenvExportedEnv(
  cwd: string | undefined,
  env: NodeJS.ProcessEnv,
  options: LoadDirenvExportedEnvOptions = {},
): NodeJS.ProcessEnv {
  const overlay = loadDirenvExportedEnv(cwd, { ...options, env: options.env ?? env });
  if (Object.keys(overlay).length === 0) return env;
  return { ...env, ...overlay };
}
