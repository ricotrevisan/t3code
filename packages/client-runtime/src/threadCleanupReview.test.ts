import { describe, expect, it } from "vite-plus/test";
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
const success = <T>(value: T) => ({ _tag: "Success" as const, value });
describe("shared cleanup review", () => {
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
    expect(controller.getSnapshot().selected).toEqual(["archive"]);
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
    await controller.execute();
    expect(controller.getSnapshot().error).toContain("Disconnected");
    expect(controller.getSnapshot().busy).toBe(false);
    await controller.reviewAgain();
    expect(controller.getSnapshot().error).toBeNull();
  });
});
