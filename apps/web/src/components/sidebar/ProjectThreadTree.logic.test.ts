import { describe, expect, it } from "vite-plus/test";
import { EnvironmentId, ProjectId, ProviderInstanceId, ThreadId } from "@t3tools/contracts";
import type { EnvironmentThreadShell } from "@t3tools/client-runtime/state/models";
import { scopeProjectRef } from "@t3tools/client-runtime/environment";
import { DEFAULT_RUNTIME_MODE } from "../../types";
import {
  buildProjectThreadGroups,
  resolveProjectDropIndicator,
  toggleAllProjectGroups,
} from "./ProjectThreadTree.logic";
import { reorderProjects, useUiStateStore } from "../../uiStateStore";

const local = EnvironmentId.make("local");
const remote = EnvironmentId.make("remote");
const ref = (environmentId = local, id = "project") =>
  scopeProjectRef(environmentId, ProjectId.make(id));
const project = (projectKey: string, memberProjectRefs = [ref()]) => ({
  projectKey,
  displayName: projectKey,
  memberProjectRefs,
});
type ThreadState = Pick<
  EnvironmentThreadShell,
  "session" | "backgroundLiveness" | "hasPendingApprovals" | "hasPendingUserInput"
>;
const thread = (title: string, projectRef = ref(), state: Partial<ThreadState> = {}) => ({
  title,
  environmentId: projectRef.environmentId,
  projectId: projectRef.projectId,
  session: null,
  backgroundLiveness: null,
  hasPendingApprovals: false,
  hasPendingUserInput: false,
  ...state,
});
const runningSession = {
  threadId: ThreadId.make("thread"),
  status: "running" as const,
  providerName: "Codex",
  providerInstanceId: ProviderInstanceId.make("codex"),
  runtimeMode: DEFAULT_RUNTIME_MODE,
  activeTurnId: null,
  lastError: null,
  updatedAt: "2026-10-05T10:00:00.000Z",
};

describe("project thread tree", () => {
  it("retains manual project order and empty projects while keeping thread priority within branches", () => {
    const groups = buildProjectThreadGroups(
      [
        project("Zebra", [ref(local, "z")]),
        project("Alpha", [ref(local, "a")]),
        project("Empty", [ref(local, "empty")]),
      ],
      [
        thread("first", ref(local, "a")),
        thread("second", ref(local, "z")),
        thread("third", ref(local, "a")),
      ],
      null,
    );
    expect(groups.map((group) => [group.label, group.threads.map((row) => row.title)])).toEqual([
      ["Zebra", ["second"]],
      ["Alpha", ["first", "third"]],
      ["Empty", []],
    ]);
  });

  it("counts waiting separately from running, including background work, without double counting", () => {
    const [group] = buildProjectThreadGroups(
      [project("Work")],
      [
        thread("ready"),
        thread("working", ref(), { session: runningSession }),
        thread("connecting", ref(), { session: { ...runningSession, status: "starting" } }),
        thread("background", ref(), { backgroundLiveness: "working" }),
        thread("monitoring", ref(), { backgroundLiveness: "monitoring" }),
        thread("input", ref(), { session: runningSession, hasPendingUserInput: true }),
        thread("approval", ref(), {
          session: runningSession,
          hasPendingUserInput: true,
          hasPendingApprovals: true,
        }),
        thread("failed", ref(), {
          session: { ...runningSession, status: "error" },
          backgroundLiveness: "working",
        }),
      ],
      null,
    );
    expect(group?.threads).toHaveLength(8);
    expect(group?.running).toBe(4);
    expect(group?.waiting).toBe(2);
  });

  it("combines logical project members across environments without conflating physical IDs", () => {
    const groups = buildProjectThreadGroups(
      [project("Grouped", [ref(), ref(remote, "other")]), project("Separate", [ref(remote)])],
      [
        thread("local"),
        thread("remote grouped", ref(remote, "other")),
        thread("remote separate", ref(remote)),
      ],
      null,
    );
    expect(groups.map((group) => group.threads.map((row) => row.title))).toEqual([
      ["local", "remote grouped"],
      ["remote separate"],
    ]);
  });

  it("limits projects to the selected scope, including an empty scoped project", () => {
    const projects = [project("Local"), project("Remote", [ref(remote)])];
    const groups = buildProjectThreadGroups(
      projects,
      [thread("outside scope")],
      new Set(["remote:project"]),
    );
    expect(groups.map((group) => group.label)).toEqual(["Remote"]);
    expect(groups[0]?.threads).toEqual([]);
  });
});

describe("project drop indicator", () => {
  const keys = ["project:a", "project:b", "project:c", "project:d"];

  it("shows the same insertion side as the saved reorder for every up/down move", () => {
    for (const active of keys) {
      for (const over of keys) {
        if (active === over) continue;
        const indicator = resolveProjectDropIndicator(keys, active, over);
        const saved = reorderProjects(useUiStateStore.getState(), keys, [active], [over]);
        expect(indicator).toEqual({
          key: over,
          position:
            saved.projectOrder.indexOf(active) < saved.projectOrder.indexOf(over)
              ? "before"
              : "after",
        });
      }
    }
  });

  it("shows no destination for cancelled, unchanged, or missing targets", () => {
    expect(resolveProjectDropIndicator(keys, "project:a", null)).toBeNull();
    expect(resolveProjectDropIndicator(keys, "project:a", "project:a")).toBeNull();
    expect(resolveProjectDropIndicator(keys, "project:missing", "project:a")).toBeNull();
    expect(resolveProjectDropIndicator(keys, "project:a", "project:missing")).toBeNull();
  });
});

describe("collapse and expand all projects", () => {
  it("collapses a partially expanded tree, then expands it without changing hidden scopes or machines", () => {
    const initial = new Set(["project:a", "project:hidden", "machine:remote"]);
    const collapsed = toggleAllProjectGroups(initial, ["project:a", "project:b"]);
    expect([...collapsed]).toEqual(["project:a", "project:hidden", "machine:remote", "project:b"]);
    expect([...initial]).toEqual(["project:a", "project:hidden", "machine:remote"]);
    const expanded = toggleAllProjectGroups(collapsed, ["project:a", "project:b"]);
    expect([...expanded]).toEqual(["project:hidden", "machine:remote"]);
  });

  it("leaves state intact when no projects are displayed", () => {
    const collapsed = new Set(["project:hidden"]);
    expect(toggleAllProjectGroups(collapsed, [])).toEqual(collapsed);
  });
});
