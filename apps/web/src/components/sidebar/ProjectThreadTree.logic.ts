import type { EnvironmentThreadShell } from "@t3tools/client-runtime/state/models";
import type { SidebarProjectSnapshot } from "../../sidebarProjectGrouping";
import { resolveSidebarThreadStatus } from "../Sidebar.logic";

type ProjectThread = Pick<
  EnvironmentThreadShell,
  | "environmentId"
  | "projectId"
  | "session"
  | "backgroundLiveness"
  | "hasPendingApprovals"
  | "hasPendingUserInput"
>;
type Project = Pick<SidebarProjectSnapshot, "projectKey" | "displayName" | "memberProjectRefs">;

/** Project order owns the tree (including empty projects); thread priority owns each branch. */
export function buildProjectThreadGroups<P extends Project, T extends ProjectThread>(
  projects: readonly P[],
  threads: readonly T[],
  scopedProjectKeys: ReadonlySet<string> | null,
) {
  const groups = projects
    .filter(
      (project) =>
        scopedProjectKeys === null ||
        project.memberProjectRefs.some((ref) =>
          scopedProjectKeys.has(`${ref.environmentId}:${ref.projectId}`),
        ),
    )
    .map((project) => ({
      key: `project:${project.projectKey}`,
      label: project.displayName,
      project,
      threads: new Array<T>(),
      running: 0,
      waiting: 0,
    }));
  const byProject = new Map(
    groups.flatMap((group) =>
      group.project.memberProjectRefs.map(
        (ref) => [`${ref.environmentId}:${ref.projectId}`, group] as const,
      ),
    ),
  );
  for (const thread of threads) {
    const group = byProject.get(`${thread.environmentId}:${thread.projectId}`);
    if (!group) continue;
    group.threads.push(thread);
    const status = resolveSidebarThreadStatus(thread);
    if (status === "working" || status === "monitoring") group.running++;
    else if (status === "input" || status === "approval") group.waiting++;
  }
  return groups;
}

/** Match the saved reorder semantics: moving down inserts after the target; moving up inserts before it. */
export function resolveProjectDropIndicator(
  keys: readonly string[],
  activeKey: string,
  overKey: string | null,
) {
  if (overKey === null || activeKey === overKey) return null;
  const activeIndex = keys.indexOf(activeKey);
  const overIndex = keys.indexOf(overKey);
  if (activeIndex < 0 || overIndex < 0) return null;
  return {
    key: overKey,
    position: activeIndex < overIndex ? ("after" as const) : ("before" as const),
  };
}

/** Toggle only the displayed projects, preserving the collapse state of hidden scopes and machines. */
export function toggleAllProjectGroups(collapsed: ReadonlySet<string>, keys: readonly string[]) {
  const next = new Set(collapsed);
  const expand = keys.every((key) => collapsed.has(key));
  for (const key of keys) {
    if (expand) next.delete(key);
    else next.add(key);
  }
  return next;
}
