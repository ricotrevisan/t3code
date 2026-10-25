import type { EnvironmentId, ThreadId } from "@t3tools/contracts";

// Navigation history is local to this app session, not unread or server state.
let lastViewedThread: { environmentId: EnvironmentId; threadId: ThreadId } | null = null;
const listeners = new Set<() => void>();

export function isLastViewedThread(environmentId: EnvironmentId, threadId: ThreadId) {
  return (
    lastViewedThread?.environmentId === environmentId && lastViewedThread.threadId === threadId
  );
}

export function recordLastViewedThread(environmentId: EnvironmentId, threadId: ThreadId) {
  if (isLastViewedThread(environmentId, threadId)) return;
  lastViewedThread = { environmentId, threadId };
  for (const listener of listeners) listener();
}

export function subscribeLastViewedThread(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
