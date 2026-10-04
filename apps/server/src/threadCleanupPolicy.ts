import type { OrchestrationThreadShell } from "@t3tools/contracts";
import { threadHasQueuedTurnStart } from "./orchestration/ThreadSettlementPolicy.ts";

/** Settlement is a review trigger, never proof that a checkout can be removed. */
export function threadCleanupBlockReason(
  thread: OrchestrationThreadShell,
  now: string,
): string | null {
  if (thread.archivedAt !== null) return "This thread is already archived.";
  if (thread.settledOverride !== "settled" || thread.settledAt === null)
    return "This thread is active. Settle it before cleaning up.";
  if (thread.latestTurn?.state === "running")
    return "This thread's turn is still running. Wait for it to finish before cleaning up.";
  if (thread.backgroundLiveness != null)
    return "Background work is still active. Wait for it to finish before cleaning up.";
  if (thread.hasPendingApprovals)
    return "This thread has a pending approval. Resolve it before cleaning up.";
  if (thread.hasPendingUserInput)
    return "This thread is waiting for user input. Resolve it before cleaning up.";
  if (threadHasQueuedTurnStart(thread, now))
    return "This thread has a queued turn. Wait for it to finish before cleaning up.";
  if (thread.session !== null && thread.session.status !== "stopped")
    return `The provider session has not stopped (${thread.session.status}). Wait for it to stop, or retry stopping it if the stop failed.`;
  return null;
}

export function cleanupReviewStillCurrent(
  reviewed: Pick<
    OrchestrationThreadShell,
    "settledAt" | "worktreePath" | "branch" | "latestUserMessageAt"
  >,
  current: OrchestrationThreadShell,
  now: string,
): boolean {
  return (
    threadCleanupBlockReason(current, now) === null &&
    reviewed.settledAt === current.settledAt &&
    reviewed.worktreePath === current.worktreePath &&
    reviewed.branch === current.branch &&
    reviewed.latestUserMessageAt === current.latestUserMessageAt
  );
}
