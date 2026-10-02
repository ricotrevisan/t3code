// @effect-diagnostics nodeBuiltinImport:off - Inspect ignored files without following symlinks.
import * as NodeFSP from "node:fs/promises";
import * as NodePath from "node:path";

/** Generated dependencies and Vite hook helpers are disposable; flag other local files. */
export async function inspectCleanupFiles(root: string, ignored: readonly string[]) {
  const files: string[] = [];
  const visited = new Set<string>();
  async function visit(relative: string): Promise<void> {
    if (NodePath.isAbsolute(relative) || relative.split(NodePath.sep).includes(".."))
      throw new Error("Cannot inspect a path outside the worktree.");
    relative = NodePath.normalize(relative);
    if (visited.has(relative)) return;
    visited.add(relative);
    if (
      relative.split(NodePath.sep).includes("node_modules") ||
      relative === NodePath.join(".vite-hooks", "_") ||
      relative.startsWith(NodePath.join(".vite-hooks", "_") + NodePath.sep)
    )
      return;
    const absolute = NodePath.join(root, relative);
    const stat = await NodeFSP.lstat(absolute);
    if (stat.isDirectory()) {
      for (const name of (await NodeFSP.readdir(absolute)).sort())
        await visit(NodePath.join(relative, name));
    } else {
      files.push(relative);
    }
    if (files.length > 10000)
      throw new Error("Too many ignored files to inspect. Preserve them manually first.");
  }
  for (const relative of ignored) await visit(relative.replace(/[\\/]+$/, ""));
  return files.sort();
}
