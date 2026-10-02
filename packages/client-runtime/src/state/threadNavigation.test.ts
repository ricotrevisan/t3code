import { describe, expect, it } from "vite-plus/test";
import { groupNavigationThreads, isThreadActivelyRunning } from "./threadNavigation.ts";

describe("isThreadActivelyRunning", () => {
  it.each([
    ["starting", true],
    ["running", true],
    ["ready", false],
    ["error", false],
  ] as const)("counts a %s session as running: %s", (status, expected) => {
    expect(
      isThreadActivelyRunning({
        session: { status },
        hasPendingApprovals: false,
        hasPendingUserInput: false,
      }),
    ).toBe(expected);
  });

  it.each([
    { hasPendingApprovals: true, hasPendingUserInput: false },
    { hasPendingApprovals: false, hasPendingUserInput: true },
    { hasPendingApprovals: true, hasPendingUserInput: true },
  ])("does not count a runtime waiting on user action %#", (flags) => {
    expect(isThreadActivelyRunning({ session: { status: "running" }, ...flags })).toBe(false);
  });

  it("does not count a missing session", () => {
    expect(
      isThreadActivelyRunning({
        session: null,
        hasPendingApprovals: false,
        hasPendingUserInput: false,
      }),
    ).toBe(false);
  });
});

describe("groupNavigationThreads", () => {
  it("keeps waiting threads visible without inflating the running count", () => {
    const rows = [
      {
        id: "working",
        session: { status: "running" as const },
        hasPendingApprovals: false,
        hasPendingUserInput: false,
      },
      {
        id: "approval",
        session: { status: "running" as const },
        hasPendingApprovals: true,
        hasPendingUserInput: false,
      },
      {
        id: "input",
        session: { status: "running" as const },
        hasPendingApprovals: false,
        hasPendingUserInput: true,
      },
    ];
    const groups = groupNavigationThreads(
      rows,
      () => ({ key: "machine", label: "Machine" }),
      isThreadActivelyRunning,
    );
    expect(groups[0]?.threads.map(({ id }) => id)).toEqual(["working", "approval", "input"]);
    expect(groups[0]?.running).toBe(1);
  });

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
