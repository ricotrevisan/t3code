import {
  UsageDay,
  type HostResourcesSnapshot,
  type UsageSummary,
  type UsageSummaryInput,
} from "@t3tools/contracts";
import * as DateTime from "effect/DateTime";

export function rollingHourWindow(nowMs: number): UsageSummaryInput {
  const until = DateTime.makeUnsafe(Math.floor(nowMs / 60_000) * 60_000);
  const since = DateTime.subtract(until, { hours: 1 });
  return {
    sinceDay: UsageDay.make(DateTime.formatIsoDateUtc(since)),
    untilDay: UsageDay.make(DateTime.formatIsoDateUtc(until)),
    timeZone: "UTC",
    resolution: "hour",
    sinceTime: DateTime.formatIso(since),
    untilTime: DateTime.formatIso(until),
  };
}

/** Disconnected, failed, and stale samples must not masquerade as live load. */
export function machineResourceValues(
  snapshot: HostResourcesSnapshot | null,
  connected: boolean,
  failed: boolean,
  nowMs: number,
) {
  const stale = !connected || failed || snapshot === null || nowMs - snapshot.sampledAt > 45_000;
  return {
    cpu: !stale && snapshot.cpuUtilization != null ? snapshot.cpuUtilization * 100 : null,
    memory:
      !stale && snapshot.totalMemoryBytes > 0
        ? (1 - snapshot.availableMemoryBytes / snapshot.totalMemoryBytes) * 100
        : null,
  };
}

/** Daily responses from older servers cannot answer a rolling-hour question. */
export function machineHourlyUsage(summary: UsageSummary | null) {
  const reporting =
    summary !== null &&
    summary.contractVersion >= 4 &&
    summary.sources.some((source) => source.status === "ok" || source.status === "partial") &&
    summary.buckets.every((bucket) => bucket.hourStart !== undefined);
  return {
    tokens: !reporting
      ? null
      : summary.buckets.reduce(
          (sum, { totals }) =>
            sum +
            totals.uncachedInputTokens +
            totals.cachedInputTokens +
            totals.cacheCreationTokens +
            totals.outputTokens,
          0,
        ),
    partial: summary?.sources.some((source) => source.status !== "ok") === true,
  };
}
