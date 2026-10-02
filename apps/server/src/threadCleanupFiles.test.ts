// @effect-diagnostics nodeBuiltinImport:off - Tests exercise real file and symlink preservation.
import { afterEach, describe, expect, it } from "vite-plus/test";
import * as NodeFSP from "node:fs/promises";
import * as NodePath from "node:path";
import * as NodeOS from "node:os";
import { inspectCleanupFiles } from "./threadCleanupFiles.ts";

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
  it("deduplicates overlapping ignored paths and flags notes and symlinks", async () => {
    const root = await fixture();
    await NodeFSP.mkdir(NodePath.join(root, "notes"));
    await NodeFSP.writeFile(NodePath.join(root, "notes", "review.md"), "Keep this evidence");
    await NodeFSP.symlink("../local.env", NodePath.join(root, ".env"));
    const files = await inspectCleanupFiles(root, ["notes/", "notes/review.md", "notes", ".env"]);
    expect(files).toEqual([".env", "notes/review.md"]);
    expect(await NodeFSP.readlink(NodePath.join(root, ".env"))).toBe("../local.env");
    expect(await NodeFSP.readFile(NodePath.join(root, "notes", "review.md"), "utf8")).toBe(
      "Keep this evidence",
    );
  });
  it("discards generated dependencies and Vite helpers but flags custom hooks", async () => {
    const root = await fixture();
    await NodeFSP.mkdir(NodePath.join(root, ".vite-hooks", "_"), { recursive: true });
    await NodeFSP.mkdir(NodePath.join(root, "node_modules"));
    await NodeFSP.writeFile(NodePath.join(root, "node_modules", "cache"), "generated");
    await NodeFSP.writeFile(NodePath.join(root, ".vite-hooks", "_", "pre-commit"), "generated");
    await NodeFSP.writeFile(NodePath.join(root, ".vite-hooks", "pre-commit"), "custom");
    const files = await inspectCleanupFiles(root, [
      "node_modules/",
      ".vite-hooks/",
      ".vite-hooks/_/pre-commit",
    ]);
    expect(files).toEqual([".vite-hooks/pre-commit"]);
  });
  it("rejects paths escaping the checkout", async () => {
    const root = await fixture();
    await expect(inspectCleanupFiles(root, ["../elsewhere"])).rejects.toThrow("outside");
  });
});
