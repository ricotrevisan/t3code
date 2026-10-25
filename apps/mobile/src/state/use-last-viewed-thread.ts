import type { EnvironmentId, ThreadId } from "@t3tools/contracts";
import { useSyncExternalStore } from "react";
import { isLastViewedThread, subscribeLastViewedThread } from "./last-viewed-thread";

export function useLastViewedThread(environmentId: EnvironmentId, threadId: ThreadId) {
  return useSyncExternalStore(subscribeLastViewedThread, () =>
    isLastViewedThread(environmentId, threadId),
  );
}
