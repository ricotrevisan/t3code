import type { SidebarThreadView } from "@t3tools/contracts";

export type ThreadNavigationView = SidebarThreadView;

/** Group an already ordered list without disturbing its saved priority. */
export function groupNavigationThreads<T>(
  threads: readonly T[],
  describe: (thread: T) => { readonly key: string; readonly label: string },
  isRunning: (thread: T) => boolean,
) {
  const groups = new Map<string, { key: string; label: string; threads: T[]; running: number }>();
  for (const thread of threads) {
    const descriptor = describe(thread);
    let group = groups.get(descriptor.key);
    if (!group) {
      group = { ...descriptor, threads: [], running: 0 };
      groups.set(group.key, group);
    }
    group.threads.push(thread);
    if (isRunning(thread)) group.running++;
  }
  return [...groups.values()].sort(
    (a, b) => a.label.localeCompare(b.label) || a.key.localeCompare(b.key),
  );
}
