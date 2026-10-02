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
  if (
    (thread.session !== null && thread.session.status !== "stopped") ||
    thread.latestTurn?.state === "running" ||
    thread.backgroundLiveness != null ||
    thread.hasPendingApprovals ||
    thread.hasPendingUserInput ||
    threadHasQueuedTurnStart(thread, now)
  )
    return "This thread still has an active session or pending work. Review again once it stops.";
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
