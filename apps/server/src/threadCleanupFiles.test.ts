// @effect-diagnostics nodeBuiltinImport:off - Tests exercise real file and symlink preservation.
import { afterEach, describe, expect, it } from "vite-plus/test";
import * as NodeFSP from "node:fs/promises";
import * as NodePath from "node:path";
import * as NodeOS from "node:os";
import { backupCleanupFiles, inspectCleanupFiles } from "./threadCleanupFiles.ts";

const roots: string[] = [];
afterEach(async () => {
  for (const root of roots.splice(0)) await NodeFSP.rm(root, { recursive: true, force: true });
});
async function fixture() {
  const root = await NodeFSP.mkdtemp(NodePath.join(NodeOS.tmpdir(), "t3-cleanup-files-"));
  roots.push(root);
  return root;
}

describe("cleanup file preservation", () => {
  it("backs up ignored notes and symlinks, excluding only dependency installations", async () => {
    const root = await fixture();
    const checkout = NodePath.join(root, "checkout");
    const backup = NodePath.join(root, "backup", "files");
    await NodeFSP.mkdir(NodePath.join(checkout, "notes"), { recursive: true });
    await NodeFSP.mkdir(NodePath.join(checkout, "node_modules"));
    await NodeFSP.writeFile(NodePath.join(checkout, "notes", "review.md"), "Keep this evidence");
    await NodeFSP.chmod(NodePath.join(checkout, "notes", "review.md"), 0o700);
    await NodeFSP.writeFile(NodePath.join(checkout, "node_modules", "cache"), "rebuildable");
    await NodeFSP.symlink("../local.env", NodePath.join(checkout, ".env"));
    const files = await inspectCleanupFiles(checkout, ["notes/", "node_modules/", ".env"]);
    expect(files.map((file) => file.path)).toEqual([".env", "notes/review.md"]);
    await backupCleanupFiles(checkout, backup, files);
    expect(await NodeFSP.readFile(NodePath.join(backup, "notes", "review.md"), "utf8")).toBe(
      "Keep this evidence",
    );
    expect(await NodeFSP.readlink(NodePath.join(backup, ".env"))).toBe("../local.env");
    expect((await NodeFSP.stat(NodePath.join(backup, "notes", "review.md"))).mode & 0o100).toBe(
      0o100,
    );
    expect(await NodeFSP.stat(NodePath.join(checkout, "notes", "review.md"))).toBeDefined();
  });
  it("refuses a stale file manifest without removing the source", async () => {
    const root = await fixture();
    const checkout = NodePath.join(root, "checkout");
    await NodeFSP.mkdir(checkout);
    await NodeFSP.writeFile(NodePath.join(checkout, "note"), "before");
    const files = await inspectCleanupFiles(checkout, ["note"]);
    await NodeFSP.writeFile(NodePath.join(checkout, "note"), "after");
    await expect(
      backupCleanupFiles(checkout, NodePath.join(root, "backup", "files"), files),
    ).rejects.toThrow("Files changed");
    expect(await NodeFSP.readFile(NodePath.join(checkout, "note"), "utf8")).toBe("after");
  });
  it("rejects paths escaping the checkout", async () => {
    const root = await fixture();
    await expect(inspectCleanupFiles(root, ["../elsewhere"])).rejects.toThrow("outside");
  });
});
