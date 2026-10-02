import { describe, expect, it } from "vite-plus/test";
import { groupNavigationThreads } from "./threadNavigation.ts";

describe("groupNavigationThreads", () => {
  it("keeps priority within groups and counts activity independently of pinning", () => {
    const rows = [
      { id: "first", machine: "b", running: false, pinned: true },
      { id: "second", machine: "a", running: true, pinned: false },
      { id: "third", machine: "b", running: true, pinned: false },
    ];
    const groups = groupNavigationThreads(
      rows,
      (row) => ({ key: row.machine, label: row.machine }),
      (row) => row.running,
    );
    expect(
      groups.map((group) => [group.key, group.threads.map((row) => row.id), group.running]),
    ).toEqual([
      ["a", ["second"], 1],
      ["b", ["first", "third"], 1],
    ]);
    expect(rows.map((row) => row.id)).toEqual(["first", "second", "third"]);
  });
  it("never merges different machines or projects just because their labels match", () => {
    const groups = groupNavigationThreads(
      ["remote:project", "local:project"],
      (key) => ({ key, label: "T3 Code" }),
      () => false,
    );
    expect(groups).toHaveLength(2);
    expect(groups.map((group) => group.key)).toEqual(["local:project", "remote:project"]);
  });
});
