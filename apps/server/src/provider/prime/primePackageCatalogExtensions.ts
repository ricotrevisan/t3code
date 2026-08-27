// @effect-diagnostics nodeBuiltinImport:off
import * as NodeFS from "node:fs";
import * as NodeOS from "node:os";
import * as NodePath from "node:path";

import { expandHomePath } from "../../pathExpansion.ts";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function resolvePrimeAgentDir(environment: NodeJS.ProcessEnv = process.env): string {
  const configured = environment.PRIME_AGENT_CODING_AGENT_DIR?.trim();
  if (configured) {
    return expandHomePath(configured);
  }
  return NodePath.join(NodeOS.homedir(), ".prime", "agent");
}

/**
 * `npm:<name>` or `npm:<name>@<version>`, including scoped names.
 * Git and path package specs are ignored: those installs are not in the
 * npm node_modules tree this helper reads.
 */
export function npmPackageNameFromSpec(spec: string): string | undefined {
  const trimmed = spec.trim();
  if (!trimmed.startsWith("npm:")) {
    return undefined;
  }
  const rest = trimmed.slice("npm:".length).trim();
  if (!rest) {
    return undefined;
  }
  if (rest.startsWith("@")) {
    const slash = rest.indexOf("/");
    if (slash <= 1 || slash === rest.length - 1) {
      return undefined;
    }
    const scope = rest.slice(0, slash);
    const nameAndVersion = rest.slice(slash + 1);
    const name = nameAndVersion.startsWith("@")
      ? nameAndVersion
      : (nameAndVersion.split("@")[0] ?? "");
    if (!scope || !name) {
      return undefined;
    }
    return `${scope}/${name}`;
  }
  const name = rest.split("@")[0] ?? "";
  return name.length > 0 ? name : undefined;
}

function readJsonFile(path: string): unknown {
  try {
    return JSON.parse(NodeFS.readFileSync(path, "utf8")) as unknown;
  } catch {
    return undefined;
  }
}

function packageSpecsFromSettings(data: unknown): ReadonlyArray<string> {
  if (!isRecord(data) || !Array.isArray(data.packages)) {
    return [];
  }
  const specs: Array<string> = [];
  for (const entry of data.packages) {
    if (typeof entry === "string") {
      const spec = entry.trim();
      if (spec) {
        specs.push(spec);
      }
      continue;
    }
    if (isRecord(entry) && typeof entry.id === "string") {
      const spec = entry.id.trim();
      if (spec) {
        specs.push(spec);
      }
    }
  }
  return specs;
}

function extensionPathsFromPackageJson(packageRoot: string, data: unknown): ReadonlyArray<string> {
  if (!isRecord(data) || !isRecord(data.pi) || !Array.isArray(data.pi.extensions)) {
    return [];
  }
  const paths: Array<string> = [];
  for (const entry of data.pi.extensions) {
    if (typeof entry !== "string") {
      continue;
    }
    const relative = entry.trim();
    if (!relative) {
      continue;
    }
    const resolved = NodePath.resolve(packageRoot, relative);
    if (NodeFS.existsSync(resolved)) {
      paths.push(resolved);
    }
  }
  return paths;
}

/**
 * Extension roots from Prime packages installed into `agentDir`.
 *
 * T3 lists Prime models with `--no-extensions` so the probe stays isolated
 * from ad-hoc user extensions. Installed packages still own provider catalogs
 * (xAI OAuth, ClinePass, …) and must be passed back explicitly via
 * `--extension`.
 */
export function resolvePrimePackageCatalogExtensionPaths(agentDir: string): ReadonlyArray<string> {
  const settings = readJsonFile(NodePath.join(agentDir, "settings.json"));
  const specs = packageSpecsFromSettings(settings);
  if (specs.length === 0) {
    return [];
  }

  const seen = new Set<string>();
  const paths: Array<string> = [];
  for (const spec of specs) {
    const packageName = npmPackageNameFromSpec(spec);
    if (!packageName) {
      continue;
    }
    const packageRoot = NodePath.join(agentDir, "npm", "node_modules", packageName);
    const packageJson = readJsonFile(NodePath.join(packageRoot, "package.json"));
    for (const extensionPath of extensionPathsFromPackageJson(packageRoot, packageJson)) {
      if (seen.has(extensionPath)) {
        continue;
      }
      seen.add(extensionPath);
      paths.push(extensionPath);
    }
  }
  return paths;
}

export function primePackageCatalogExtensionArgs(agentDir: string): ReadonlyArray<string> {
  return resolvePrimePackageCatalogExtensionPaths(agentDir).flatMap((extensionPath) => [
    "--extension",
    extensionPath,
  ]);
}
