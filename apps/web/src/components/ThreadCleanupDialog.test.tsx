// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import { AsyncResult } from "effect/unstable/reactivity";
import {
  EnvironmentId,
  ThreadId,
  ProjectId,
  ProviderInstanceId,
  type OrchestrationThreadShell,
} from "@t3tools/contracts";
import { requestThreadCleanup, ThreadCleanupDialogHost } from "./ThreadCleanupDialog";

const mocks = vi.hoisted(() => ({
  shell: null as OrchestrationThreadShell | null,
  listeners: new Set<() => void>(),
  inspect: vi.fn(),
  run: vi.fn(),
}));
vi.mock("../state/threads", () => ({
  threadEnvironment: { reviewCleanup: "inspect", runCleanup: "run" },
}));
vi.mock("../state/use-atom-command", () => ({
  useAtomCommand: (command: string) => (command === "inspect" ? mocks.inspect : mocks.run),
}));
vi.mock("../state/entities", async () => {
  const { useSyncExternalStore } = await import("react");
  return {
    useThreadShell: () =>
      useSyncExternalStore(
        (listener) => {
          mocks.listeners.add(listener);
          return () => {
            mocks.listeners.delete(listener);
          };
        },
        () => mocks.shell,
      ),
  };
});
let root: Root;
let container: HTMLDivElement;
const target = {
  environmentId: EnvironmentId.make("fixture-env"),
  threadId: ThreadId.make("fixture-thread"),
};
const now = "2026-01-01T00:00:00.000Z";
const fixture: OrchestrationThreadShell = {
  id: target.threadId,
  projectId: ProjectId.make("fixture-project"),
  title: "Finished fixture",
  modelSelection: { instanceId: ProviderInstanceId.make("codex"), model: "gpt-5.4" },
  runtimeMode: "full-access",
  interactionMode: "default",
  branch: "task",
  worktreePath: "/fixture-only",
  pullRequests: [],
  latestTurn: null,
  createdAt: now,
  updatedAt: now,
  archivedAt: null,
  settledOverride: "settled",
  settledAt: now,
  latestUserMessageAt: null,
  hasPendingApprovals: false,
  hasPendingUserInput: false,
  hasActionableProposedPlan: false,
  session: {
    threadId: target.threadId,
    status: "ready",
    providerName: "codex",
    runtimeMode: "full-access",
    activeTurnId: null,
    lastError: null,
    updatedAt: now,
  },
};
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  mocks.shell = fixture;
  mocks.inspect.mockReset();
  mocks.run.mockReset();
  mocks.inspect.mockImplementation(async () =>
    AsyncResult.success({
      reviewId: `review-${mocks.inspect.mock.calls.length}`,
      threadId: target.threadId,
      title: "Finished fixture",
      reviewedAt: now,
      actions: [
        {
          id: "worktree",
          title: "Remove worktree",
          detail: "Fixture",
          blockedReason: "The provider session has not stopped (ready).",
        },
        {
          id: "archive",
          title: "Archive this thread",
          detail: "Fixture",
          blockedReason: "The provider session has not stopped (ready).",
        },
      ],
    }),
  );
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});
async function project(
  patch: Partial<OrchestrationThreadShell>,
  blockedReason: string | null = null,
) {
  mocks.inspect.mockResolvedValue(
    AsyncResult.success({
      reviewId: `review-${mocks.inspect.mock.calls.length + 1}`,
      threadId: target.threadId,
      title: "Finished fixture",
      reviewedAt: now,
      actions: [
        { id: "worktree", title: "Remove worktree", detail: "Fixture", blockedReason },
        { id: "archive", title: "Archive this thread", detail: "Fixture", blockedReason },
      ],
    }),
  );
  await act(async () => {
    mocks.shell = { ...mocks.shell!, ...patch };
    for (const listener of mocks.listeners) listener();
  });
}
function runButton() {
  return Array.from(document.querySelectorAll("button")).find((b) =>
    b.textContent?.startsWith("Run "),
  )!;
}
it("the mounted cleanup dialog unlocks after the stop projection, preserves choices, and never auto-runs", async () => {
  await act(async () => {
    requestThreadCleanup(target);
    root.render(<ThreadCleanupDialogHost />);
  });
  expect(document.body.textContent).toContain("provider session has not stopped (ready)");
  expect(runButton().disabled).toBe(true);
  expect(mocks.inspect).toHaveBeenCalledOnce();
  await project({ session: { ...fixture.session!, status: "stopped" } });
  expect(document.body.textContent).not.toContain("provider session has not stopped");
  expect(runButton().textContent).toBe("Run 1 selected");
  expect(runButton().disabled).toBe(false);
  const checkbox = document.querySelector<HTMLButtonElement>('[aria-label="Remove worktree"]')!;
  await act(async () => checkbox.click());
  expect(runButton().textContent).toBe("Run 0 selected");
  await project({ hasPendingApprovals: true }, "This thread has a pending approval.");
  expect(document.body.textContent).toContain("pending approval");
  expect(runButton().disabled).toBe(true);
  await project({ hasPendingApprovals: false });
  expect(runButton().textContent).toBe("Run 0 selected");
  expect(mocks.inspect).toHaveBeenCalledTimes(4);
  expect(mocks.run).not.toHaveBeenCalled();
});
