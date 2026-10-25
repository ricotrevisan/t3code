import { describe, expect, it } from "@effect/vitest";
import { EnvironmentId, ThreadId } from "@t3tools/contracts";
import {
  isLastViewedThread,
  recordLastViewedThread,
  subscribeLastViewedThread,
} from "./last-viewed-thread";

describe("last viewed thread", () => {
  it("retains one scoped destination until another thread is viewed", () => {
    const first = EnvironmentId.make("first");
    const second = EnvironmentId.make("second");
    const thread = ThreadId.make("thread");
    expect(isLastViewedThread(first, thread)).toBe(false);
    let notifications = 0;
    const unsubscribe = subscribeLastViewedThread(() => {
      notifications += 1;
    });
    recordLastViewedThread(first, thread);
    expect(isLastViewedThread(first, thread)).toBe(true);
    expect(isLastViewedThread(second, thread)).toBe(false);
    recordLastViewedThread(first, thread);
    expect(notifications).toBe(1);
    recordLastViewedThread(second, thread);
    expect(isLastViewedThread(first, thread)).toBe(false);
    expect(isLastViewedThread(second, thread)).toBe(true);
    expect(notifications).toBe(2);
    unsubscribe();
    recordLastViewedThread(first, thread);
    expect(notifications).toBe(2);
  });
});
