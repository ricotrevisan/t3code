import { AsyncResult } from "effect/unstable/reactivity";
import { afterEach, describe, expect, it, vi } from "vite-plus/test";
import {
  EnvironmentId,
  ThreadId,
  type ThreadCleanupReview,
  type ThreadCleanupResult,
} from "@t3tools/contracts";
import { createThreadCleanupReview } from "./threadCleanupReview.ts";

const target = { environmentId: EnvironmentId.make("env"), threadId: ThreadId.make("thread") };
const review: ThreadCleanupReview = {
  reviewId: "review-1",
  threadId: target.threadId,
  title: "Finished",
  reviewedAt: "2026-01-01T00:00:00.000Z",
  actions: [
    { id: "worktree", title: "Worktree", detail: "", blockedReason: "Shared" },
    { id: "archive", title: "Archive", detail: "", blockedReason: null },
  ],
};
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}
const success = <T>(value: T) => AsyncResult.success(value);
describe("shared cleanup review", () => {
  afterEach(() => vi.useRealTimers());
  it("dismisses an untouched review after 15 seconds without running selected actions", async () => {
    vi.useFakeTimers();
    const dismiss = vi.fn();
    const run = vi.fn(async () => success({ threadId: target.threadId, actions: [] }));
    const controller = createThreadCleanupReview(target, {
      inspect: async () =>
        success({
          ...review,
          actions: review.actions.map((action) => ({ ...action, blockedReason: null })),
        }),
      run,
      dismiss,
    });
    await controller.reviewAgain();
    expect(controller.getSnapshot().remainingSeconds).toBe(15);
    expect(controller.getSnapshot().selected).toEqual(["worktree"]);
    await vi.advanceTimersByTimeAsync(14_000);
    expect(controller.getSnapshot().remainingSeconds).toBe(1);
    expect(dismiss).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1_000);
    expect(dismiss).toHaveBeenCalledOnce();
    expect(run).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
    controller.dispose();
  });
  it("stops the countdown on interaction and never dismisses an in-progress cleanup", async () => {
    vi.useFakeTimers();
    const pending = deferred<ReturnType<typeof success<ThreadCleanupResult>>>();
    const dismiss = vi.fn();
    const controller = createThreadCleanupReview(target, {
      inspect: async () => success(review),
      run: () => pending.promise,
      dismiss,
    });
    await controller.reviewAgain();
    controller.toggle("archive", true);
    expect(controller.getSnapshot().remainingSeconds).toBeNull();
    await vi.advanceTimersByTimeAsync(30_000);
    expect(dismiss).not.toHaveBeenCalled();
    const running = controller.execute();
    controller.dismiss();
    expect(dismiss).not.toHaveBeenCalled();
    pending.resolve(success({ threadId: target.threadId, actions: [] }));
    await running;
    await vi.advanceTimersByTimeAsync(30_000);
    expect(dismiss).not.toHaveBeenCalled();
    controller.dismiss();
    expect(dismiss).toHaveBeenCalledOnce();
    controller.dispose();
  });
  it("dismissal cancels a pending review and disposal clears its timer", async () => {
    vi.useFakeTimers();
    const pending = deferred<ReturnType<typeof success<ThreadCleanupReview>>>();
    const dismiss = vi.fn();
    const run = vi.fn(async () => success({ threadId: target.threadId, actions: [] }));
    const controller = createThreadCleanupReview(target, {
      inspect: () => pending.promise,
      run,
      dismiss,
    });
    const inspecting = controller.reviewAgain();
    controller.dismiss();
    pending.resolve(success(review));
    await inspecting;
    expect(controller.getSnapshot().review).toBeNull();
    expect(dismiss).toHaveBeenCalledOnce();
    expect(run).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
    await controller.reviewAgain();
    controller.dispose();
    expect(vi.getTimerCount()).toBe(0);
  });
  it("leaves archive unchecked on initial review and subsequent reviews", async () => {
    const controller = createThreadCleanupReview(target, {
      inspect: async () =>
        success({
          ...review,
          actions: [
            ...review.actions.map((action) => ({ ...action, blockedReason: null })),
            { id: "remote-branch", title: "Delete remote branch", detail: "", blockedReason: null },
          ],
        }),
      run: async () => success({ threadId: target.threadId, actions: [] }),
    });
    await controller.reviewAgain();
    expect(controller.getSnapshot().selected).toEqual(["worktree", "remote-branch"]);
    controller.toggle("archive", true);
    expect(controller.getSnapshot().selected).toEqual(["worktree", "remote-branch", "archive"]);
    await controller.reviewAgain();
    expect(controller.getSnapshot().selected).toEqual(["worktree", "remote-branch"]);
  });
  it("executes only selected eligible actions and prevents a duplicate confirmation", async () => {
    const pending = deferred<ReturnType<typeof success<ThreadCleanupResult>>>();
    const calls: unknown[] = [];
    const controller = createThreadCleanupReview(target, {
      inspect: async () => success(review),
      run: (input) => {
        calls.push(input);
        return pending.promise;
      },
    });
    await controller.reviewAgain();
    controller.toggle("worktree", true);
    expect(controller.getSnapshot().selected).toEqual([]);
    controller.toggle("archive", false);
    await controller.execute();
    expect(calls).toHaveLength(0);
    controller.toggle("archive", true);
    const first = controller.execute();
    await controller.execute();
    controller.toggle("archive", false);
    expect(calls).toEqual([
      {
        environmentId: target.environmentId,
        input: { threadId: target.threadId, reviewId: "review-1", selected: ["archive"] },
      },
    ]);
    expect(controller.getSnapshot().busy).toBe(true);
    pending.resolve(
      success({
        threadId: target.threadId,
        actions: [{ id: "archive", status: "completed", detail: "Archived" }],
      }),
    );
    await first;
    expect(controller.getSnapshot().result?.actions[0]?.status).toBe("completed");
    expect(controller.getSnapshot().busy).toBe(false);
  });
  it("ignores an older review response after a new review arrives", async () => {
    const old = deferred<ReturnType<typeof success<ThreadCleanupReview>>>();
    let calls = 0;
    const controller = createThreadCleanupReview(target, {
      inspect: async () => (++calls === 1 ? old.promise : success({ ...review, reviewId: "new" })),
      run: async () => success({ threadId: target.threadId, actions: [] }),
    });
    const first = controller.reviewAgain();
    await controller.reviewAgain();
    old.resolve(success(review));
    await first;
    expect(controller.getSnapshot().review?.reviewId).toBe("new");
  });
  it("allows retry after an unexpected transport failure", async () => {
    const controller = createThreadCleanupReview(target, {
      inspect: async () => success(review),
      run: async () => {
        throw new Error("Disconnected");
      },
    });
    await controller.reviewAgain();
    controller.toggle("archive", true);
    await controller.execute();
    expect(controller.getSnapshot().error).toContain("Disconnected");
    expect(controller.getSnapshot().busy).toBe(false);
    await controller.reviewAgain();
    expect(controller.getSnapshot().error).toBeNull();
  });
});
