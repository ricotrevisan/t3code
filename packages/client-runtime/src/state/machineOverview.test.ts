import { describe, expect, it } from "vite-plus/test";
import { UsageDay, UsageSummary } from "@t3tools/contracts";
import * as Schema from "effect/Schema";
import { machineHourlyUsage, machineResourceValues, rollingHourWindow } from "./machineOverview.ts";

const now = Date.parse("2026-10-02T00:20:34Z");
const summary = Schema.decodeSync(UsageSummary)({
  contractVersion: 6,
  readAt: "2026-10-02T00:20:00Z",
  timeZone: "UTC",
  sinceDay: UsageDay.make("2026-10-01"),
  untilDay: UsageDay.make("2026-10-02"),
  sources: [
    {
      fingerprint: {
        hostId: "lab",
        provider: "codex",
        resolvedHomePath: "/codex",
        volumeId: "1:2",
      },
      status: "ok",
      scannedFiles: 1,
      skippedFiles: 0,
      malformedRecords: 0,
      distinctSessions: 1,
      message: null,
    },
  ],
  buckets: [
    {
      day: UsageDay.make("2026-10-02"),
      hourStart: "2026-10-02T00:00:00Z",
      provider: "codex",
      model: "test",
      totals: {
        uncachedInputTokens: 10,
        cachedInputTokens: 20,
        cacheCreationTokens: 30,
        outputTokens: 40,
        reasoningTokens: 15,
      },
      costUsd: 0,
      cacheSavingsUsd: 0,
      costSource: "unpriced",
      records: 1,
      unpricedRecords: 1,
      sessions: 1,
    },
  ],
  pricing: { status: "unavailable", source: "test", fetchedAt: null, knownModels: 0 },
  scanDurationMs: 1,
});

describe("machine overview", () => {
  it("uses a stable rolling hour across midnight", () => {
    expect(rollingHourWindow(now)).toMatchObject({
      sinceDay: "2026-10-01",
      untilDay: "2026-10-02",
      sinceTime: "2026-10-01T23:20:00.000Z",
      untilTime: "2026-10-02T00:20:00.000Z",
      resolution: "hour",
      timeZone: "UTC",
    });
  });
  it("never double-counts reasoning as extra output", () => {
    expect(machineHourlyUsage(summary)).toEqual({ tokens: 100, partial: false });
  });
  it("distinguishes zero measured usage from unavailable usage and older daily responses", () => {
    expect(machineHourlyUsage({ ...summary, buckets: [] }).tokens).toBe(0);
    expect(machineHourlyUsage({ ...summary, sources: [] }).tokens).toBeNull();
    expect(
      machineHourlyUsage({
        ...summary,
        buckets: summary.buckets.map((bucket) => ({ ...bucket, hourStart: undefined })),
      }).tokens,
    ).toBeNull();
    expect(machineHourlyUsage(null).tokens).toBeNull();
  });
  it("marks incomplete provider coverage", () => {
    expect(
      machineHourlyUsage({
        ...summary,
        sources: summary.sources.map((source) => ({ ...source, status: "partial" })),
      }),
    ).toEqual({ tokens: 100, partial: true });
  });
  it("hides load when disconnected, stale, failed, or missing", () => {
    const host = {
      sampledAt: now,
      cpuUtilization: 0.25,
      cpuCount: 4,
      availableMemoryBytes: 25,
      totalMemoryBytes: 100,
    };
    expect(machineResourceValues(host, true, false, now)).toEqual({ cpu: 25, memory: 75 });
    for (const values of [
      machineResourceValues(host, false, false, now),
      machineResourceValues(host, true, true, now),
      machineResourceValues(host, true, false, now + 45_001),
      machineResourceValues(null, true, false, now),
    ]) {
      expect(values).toEqual({ cpu: null, memory: null });
    }
  });
});
