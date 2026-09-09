import { describe, expect, it } from "vite-plus/test";
import * as Schema from "effect/Schema";

import {
  USAGE_CONTRACT_VERSION,
  UsageBucket,
  UsageSourceFingerprint,
  UsageSummary,
} from "./usage.ts";

const decodeUsageBucket = Schema.decodeUnknownSync(UsageBucket);
const decodeUsageSourceFingerprint = Schema.decodeUnknownSync(UsageSourceFingerprint);
const decodeUsageSummary = Schema.decodeUnknownSync(UsageSummary);

describe("usage contract v6", () => {
  it("decodes a v5 bucket that omits harness as native-shaped", () => {
    const decoded = decodeUsageBucket({
      day: "2026-08-07",
      provider: "codex",
      model: "gpt-5.6-sol",
      totals: {
        uncachedInputTokens: 1,
        cachedInputTokens: 0,
        cacheCreationTokens: 0,
        outputTokens: 2,
        reasoningTokens: 0,
      },
      costUsd: 0,
      cacheSavingsUsd: 0,
      costSource: "unpriced",
      records: 1,
      unpricedRecords: 1,
      sessions: 1,
    });

    expect(decoded.harness).toBeUndefined();
    expect(decoded.provider).toBe("codex");
  });

  it("accepts opencode, unknown, and primeAgent harness on current buckets", () => {
    const decoded = decodeUsageBucket({
      day: "2026-08-07",
      provider: "opencode",
      harness: "primeAgent",
      model: "glm-5.3-flash",
      totals: {
        uncachedInputTokens: 10,
        cachedInputTokens: 4,
        cacheCreationTokens: 2,
        outputTokens: 3,
        reasoningTokens: 0,
      },
      costUsd: 0,
      cacheSavingsUsd: 0,
      costSource: "unpriced",
      records: 1,
      unpricedRecords: 1,
      sessions: 1,
    });

    expect(decoded.provider).toBe("opencode");
    expect(decoded.harness).toBe("primeAgent");
  });

  it("decodes an older source fingerprint that omits harness", () => {
    const decoded = decodeUsageSourceFingerprint({
      hostId: "mac",
      provider: "claude",
      resolvedHomePath: "/home/theo/.claude",
      volumeId: "1:2",
    });

    expect(decoded.harness).toBeUndefined();
  });

  it("round-trips a current summary", () => {
    const summary = decodeUsageSummary({
      contractVersion: USAGE_CONTRACT_VERSION,
      readAt: "2026-08-07T00:00:00.000Z",
      timeZone: "UTC",
      sinceDay: "2026-08-01",
      untilDay: "2026-08-31",
      buckets: [
        {
          day: "2026-08-07",
          provider: "unknown",
          harness: "primeAgent",
          model: "mystery",
          totals: {
            uncachedInputTokens: 1,
            cachedInputTokens: 0,
            cacheCreationTokens: 0,
            outputTokens: 0,
            reasoningTokens: 0,
          },
          costUsd: 0,
          cacheSavingsUsd: 0,
          costSource: "unpriced",
          records: 1,
          unpricedRecords: 1,
          sessions: 1,
        },
      ],
      sources: [
        {
          fingerprint: {
            hostId: "mac",
            provider: "unknown",
            harness: "primeAgent",
            resolvedHomePath: "/env/prime-agent/sessions",
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
      pricing: { status: "fresh", source: "litellm", fetchedAt: null, knownModels: 1 },
      scanDurationMs: 1,
    });

    expect(summary.contractVersion).toBe(USAGE_CONTRACT_VERSION);
    expect(summary.buckets[0]?.provider).toBe("unknown");
    expect(summary.sources[0]?.fingerprint.harness).toBe("primeAgent");
  });
});
