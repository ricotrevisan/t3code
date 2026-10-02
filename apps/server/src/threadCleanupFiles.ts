// @effect-diagnostics nodeBuiltinImport:off - lstat and symlink-preserving backup verification.
import * as NodeFSP from "node:fs/promises";
import * as NodePath from "node:path";
import * as NodeCrypto from "node:crypto";

const MAX_BACKUP_BYTES = 512 * 1024 * 1024;
export interface CleanupFile {
  readonly path: string;
  readonly kind: "file" | "symlink";
  readonly fingerprint: string;
  readonly bytes: number;
  readonly executable: boolean;
}

/** Only dependency installations are discarded; every other ignored file is preserved. */
export async function inspectCleanupFiles(root: string, ignored: readonly string[]) {
  const files: CleanupFile[] = [];
  let bytes = 0;
  async function visit(relative: string): Promise<void> {
    if (relative.split(NodePath.sep).includes("node_modules")) return;
    if (NodePath.isAbsolute(relative) || relative.split(NodePath.sep).includes(".."))
      throw new Error("Cannot back up a path outside the worktree.");
    const absolute = NodePath.join(root, relative);
    const stat = await NodeFSP.lstat(absolute);
    if (stat.isSymbolicLink()) {
      files.push({
        path: relative,
        kind: "symlink",
        fingerprint: await NodeFSP.readlink(absolute),
        bytes: 0,
        executable: false,
      });
    } else if (stat.isDirectory()) {
      for (const name of (await NodeFSP.readdir(absolute)).sort())
        await visit(NodePath.join(relative, name));
    } else if (stat.isFile()) {
      bytes += stat.size;
      if (bytes > MAX_BACKUP_BYTES)
        throw new Error(
          "Ignored files exceed 512 MB. Preserve them manually before removing this worktree.",
        );
      const fingerprint = NodeCrypto.createHash("sha256")
        .update(await NodeFSP.readFile(absolute))
        .digest("hex");
      files.push({
        path: relative,
        kind: "file",
        fingerprint,
        bytes: stat.size,
        executable: (stat.mode & 0o111) !== 0,
      });
    } else throw new Error(`Preserve the special file ${relative} manually before cleanup.`);
    if (files.length > 10000)
      throw new Error("Too many ignored files to back up. Preserve them manually first.");
  }
  for (const relative of [...new Set(ignored)].sort()) await visit(relative.replace(/[\\/]+$/, ""));
  return files.sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
}

/** Re-read both sides: a successful copy alone does not prove preservation. */
export async function backupCleanupFiles(
  root: string,
  destination: string,
  files: readonly CleanupFile[],
) {
  await NodeFSP.mkdir(destination, { recursive: true, mode: 0o700 });
  for (const file of files) {
    const target = NodePath.join(destination, file.path);
    await NodeFSP.mkdir(NodePath.dirname(target), { recursive: true, mode: 0o700 });
    if (file.kind === "symlink") await NodeFSP.symlink(file.fingerprint, target);
    else {
      await NodeFSP.copyFile(NodePath.join(root, file.path), target);
      await NodeFSP.chmod(target, file.executable ? 0o700 : 0o600);
    }
  }
  const paths = files.map((file) => file.path);
  const [source, backup] = await Promise.all([
    inspectCleanupFiles(root, paths),
    inspectCleanupFiles(destination, paths),
  ]);
  if (!sameCleanupFiles(source, files) || !sameCleanupFiles(backup, files))
    throw new Error("Files changed during backup. The worktree was kept.");
  await NodeFSP.writeFile(
    NodePath.join(NodePath.dirname(destination), "cleanup-manifest.json"),
    JSON.stringify({ root, files }, null, 2),
    { mode: 0o600 },
  );
}

export function sameCleanupFiles(left: readonly CleanupFile[], right: readonly CleanupFile[]) {
  return (
    left.length === right.length &&
    left.every((file, i) => {
      const other = right[i];
      return (
        other &&
        file.path === other.path &&
        file.kind === other.kind &&
        file.fingerprint === other.fingerprint &&
        file.bytes === other.bytes &&
        file.executable === other.executable
      );
    })
  );
}
