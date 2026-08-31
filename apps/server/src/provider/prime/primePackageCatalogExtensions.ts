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
    // A filtered package entry owns explicit enable/disable semantics. T3
    // cannot safely reconstruct those while running Prime with
    // `--no-extensions`, so only reattach object-form packages that leave
    // extension autoloading at its default.
    if (
      isRecord(entry) &&
      typeof entry.source === "string" &&
      entry.autoload !== false &&
      entry.extensions === undefined
    ) {
      const spec = entry.source.trim();
      if (spec) {
        specs.push(spec);
      }
    }
  }
  return specs;
}

function isExtensionFileName(name: string): boolean {
  return name.endsWith(".ts") || name.endsWith(".js");
}

/**
 * Prime `--extension` loads files, not directory roots. Expand a package.json
 * `pi.extensions` entry the same way Prime's package loader does: a file is
 * used as-is, a directory contributes `index.ts`/`index.js` or top-level
 * `.ts`/`.js` files (nested helpers without an index are skipped).
 */
function extensionFilesFromManifestEntry(
  packageRoot: string,
  relative: string,
): ReadonlyArray<string> {
  const resolved = NodePath.resolve(packageRoot, relative);
  let stats: NodeFS.Stats;
  try {
    stats = NodeFS.statSync(resolved);
  } catch {
    return [];
  }
  if (stats.isFile()) {
    return [resolved];
  }
  if (!stats.isDirectory()) {
    return [];
  }

  const indexTs = NodePath.join(resolved, "index.ts");
  if (NodeFS.existsSync(indexTs)) {
    return [indexTs];
  }
  const indexJs = NodePath.join(resolved, "index.js");
  if (NodeFS.existsSync(indexJs)) {
    return [indexJs];
  }

  const files: Array<string> = [];
  let entries: ReadonlyArray<NodeFS.Dirent>;
  try {
    entries = NodeFS.readdirSync(resolved, { withFileTypes: true });
  } catch {
    return [];
  }
  for (const entry of entries) {
    if (entry.name.startsWith(".") || entry.name === "node_modules") {
      continue;
    }
    const fullPath = NodePath.join(resolved, entry.name);
    if (entry.isFile() && isExtensionFileName(entry.name)) {
      files.push(fullPath);
      continue;
    }
    if (!entry.isDirectory()) {
      continue;
    }
    const nestedIndexTs = NodePath.join(fullPath, "index.ts");
    if (NodeFS.existsSync(nestedIndexTs)) {
      files.push(nestedIndexTs);
      continue;
    }
    const nestedIndexJs = NodePath.join(fullPath, "index.js");
    if (NodeFS.existsSync(nestedIndexJs)) {
      files.push(nestedIndexJs);
    }
  }
  return files;
}

function isOverridePattern(entry: string): boolean {
  return entry.startsWith("!") || entry.startsWith("+") || entry.startsWith("-");
}

function matchesManifestPattern(filePath: string, pattern: string, packageRoot: string): boolean {
  const normalized = pattern.startsWith("./") ? pattern.slice(2) : pattern;
  const relative = NodePath.relative(packageRoot, filePath).split(NodePath.sep).join("/");
  const absolute = filePath.split(NodePath.sep).join("/");
  try {
    return (
      NodePath.matchesGlob(relative, normalized) ||
      NodePath.matchesGlob(NodePath.basename(filePath), normalized) ||
      NodePath.matchesGlob(absolute, normalized)
    );
  } catch {
    return false;
  }
}

function matchesExactManifestPath(filePath: string, pattern: string, packageRoot: string): boolean {
  const normalized = (pattern.startsWith("./") ? pattern.slice(2) : pattern)
    .split(NodePath.sep)
    .join("/");
  const relative = NodePath.relative(packageRoot, filePath).split(NodePath.sep).join("/");
  const absolute = filePath.split(NodePath.sep).join("/");
  return normalized === relative || normalized === absolute;
}

function applyManifestOverrides(
  files: ReadonlyArray<string>,
  entries: ReadonlyArray<string>,
  packageRoot: string,
): ReadonlyArray<string> {
  const excludes = entries.filter((entry) => entry.startsWith("!")).map((entry) => entry.slice(1));
  const forceIncludes = entries
    .filter((entry) => entry.startsWith("+"))
    .map((entry) => entry.slice(1));
  const forceExcludes = entries
    .filter((entry) => entry.startsWith("-"))
    .map((entry) => entry.slice(1));
  const enabled = files.filter(
    (filePath) =>
      !excludes.some((pattern) => matchesManifestPattern(filePath, pattern, packageRoot)),
  );
  for (const filePath of files) {
    if (
      !enabled.includes(filePath) &&
      forceIncludes.some((pattern) => matchesExactManifestPath(filePath, pattern, packageRoot))
    ) {
      enabled.push(filePath);
    }
  }
  return enabled.filter(
    (filePath) =>
      !forceExcludes.some((pattern) => matchesExactManifestPath(filePath, pattern, packageRoot)),
  );
}

function extensionFilesFromPackageJson(packageRoot: string, data: unknown): ReadonlyArray<string> {
  if (!isRecord(data)) {
    return [];
  }
  if (!isRecord(data.pi)) {
    return extensionFilesFromManifestEntry(packageRoot, "extensions");
  }
  if (!Array.isArray(data.pi.extensions)) {
    return [];
  }
  const entries = data.pi.extensions
    .filter((entry): entry is string => typeof entry === "string")
    .map((entry) => entry.trim())
    .filter(Boolean);
  const files = entries
    .filter((entry) => !isOverridePattern(entry))
    .flatMap((entry) => {
      const matches =
        entry.includes("*") || entry.includes("?")
          ? NodeFS.globSync(entry, { cwd: packageRoot }).map((match) =>
              NodePath.resolve(packageRoot, match),
            )
          : [entry];
      return matches.flatMap((match) => extensionFilesFromManifestEntry(packageRoot, match));
    });
  return applyManifestOverrides(files, entries, packageRoot);
}

/**
 * Extension files from Prime packages installed into `agentDir`.
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
    for (const extensionPath of extensionFilesFromPackageJson(packageRoot, packageJson)) {
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
