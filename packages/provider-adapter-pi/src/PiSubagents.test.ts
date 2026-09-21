// @effect-diagnostics nodeBuiltinImport:off
import * as NodeFSP from "node:fs/promises";
import * as NodeOS from "node:os";
import * as NodePath from "node:path";
import { describe, expect, it } from "vite-plus/test";
import { makePiSubagents } from "./PiSubagents.ts";

const child = (overrides: Record<string, unknown> = {}) => ({
  agent: "worker",
  prompt: "Check the baseline",
  session: { id: "baseline", handle: "baseline" },
  exitCode: -1,
  messages: [],
  usage: { input: 10, output: 2, cacheRead: 4, cacheWrite: 1 },
  ...overrides,
});
const snapshot = (...results: unknown[]) => ({ details: { kind: "pi-subagent", results } });

describe("Pi subagent snapshots", () => {
  it("ignores unrelated tools and skips malformed children without losing valid siblings", () => {
    const tracker = makePiSubagents();
    expect(tracker.observe("call", { details: { results: [child()] } })).toEqual([]);
    const events = tracker.observe("call", snapshot(null, { agent: "broken" }, child()));
    expect(events.map((event) => event.type)).toEqual([
      "task.started",
      "task.updated",
      "task.progress",
    ]);
    expect(events[0]?.payload.taskId).toBe("pi:baseline");
  });

  it("deduplicates heartbeats and reads tool activity without carrying child transcripts", () => {
    const tracker = makePiSubagents();
    const value = snapshot(
      child({
        messages: [
          {
            role: "assistant",
            content: [
              { type: "text", text: "x".repeat(100_000) },
              { type: "toolCall", name: "read" },
            ],
          },
        ],
      }),
    );
    const events = tracker.observe("call", value);
    expect(events[2]?.payload).toMatchObject({
      lastToolName: "read",
      typedUsage: { totalTokens: 17 },
    });
    expect(JSON.stringify(events).length).toBeLessThan(2000);
    expect(tracker.observe("call", value)).toEqual([]);
  });

  it("settles cancelled children and does not rewrite completed siblings on transport closure", () => {
    const tracker = makePiSubagents();
    tracker.observe("call", snapshot(child(), child({ session: { id: "other" }, exitCode: 0 })));
    const events = tracker.interrupt();
    expect(events).toHaveLength(1);
    expect(events[0]?.payload).toMatchObject({ taskId: "pi:baseline", status: "stopped" });
    expect(tracker.interrupt()).toEqual([]);
  });

  it("handles mixed batch outcomes independently and keeps terminal snapshots idempotent", () => {
    const tracker = makePiSubagents();
    tracker.observe("call", snapshot(child()));
    const value = snapshot(
      child({ exitCode: 130, stopReason: "aborted", errorMessage: "Cancelled" }),
    );
    expect(tracker.observe("call", value)[0]?.payload).toMatchObject({
      status: "stopped",
      summary: "Cancelled",
    });
    expect(tracker.observe("call", value)).toEqual([]);
    expect(tracker.finish("call", "failed", "Tool failed")).toEqual([]);
  });

  it("reactivates a named session even when only a final result is available", () => {
    const tracker = makePiSubagents();
    const value = snapshot(child({ exitCode: 0 }));
    tracker.observe("first", value);
    tracker.finish("first", "failed", "Missing result");
    const events = tracker.observe("second", value);
    expect(events.map((event) => event.type)).toEqual([
      "task.started",
      "task.updated",
      "task.completed",
    ]);
    expect(events[1]?.payload).toMatchObject({ status: "running" });
    expect(events[2]?.payload).toMatchObject({
      taskId: "pi:baseline",
      typedUsage: { totalTokens: 34 },
    });
  });

  it("restores cumulative usage from native tool results without double-counting duplicates", () => {
    const result = {
      role: "toolResult",
      toolName: "subagent",
      toolCallId: "first",
      ...snapshot(child({ exitCode: 0 })),
    };
    const tracker = makePiSubagents();
    tracker.restore([result, result, { ...result, toolName: "other" }]);
    const events = tracker.observe("second", snapshot(child({ exitCode: 0 })));
    expect(events.at(-1)?.payload).toMatchObject({ typedUsage: { totalTokens: 34 } });
  });

  it("streams native history, ignoring unrelated and incomplete records", async () => {
    const dir = await NodeFSP.mkdtemp(NodePath.join(NodeOS.tmpdir(), "pi-subagent-history-"));
    try {
      const path = NodePath.join(dir, "session.jsonl");
      const result = (toolCallId: string) =>
        JSON.stringify({
          type: "message",
          message: {
            role: "toolResult",
            toolName: "subagent",
            toolCallId,
            ...snapshot(child({ exitCode: 0 })),
          },
        });
      await NodeFSP.writeFile(
        path,
        [
          result("first"),
          ...Array.from({ length: 1000 }, () =>
            JSON.stringify({
              type: "message",
              message: { role: "user", content: "x".repeat(1024) },
            }),
          ),
          result("second"),
          result("second"),
          '{"partial":',
        ].join("\n"),
      );
      const tracker = makePiSubagents();
      await tracker.restoreFile(path);
      expect(
        tracker.observe("third", snapshot(child({ exitCode: 0 }))).at(-1)?.payload,
      ).toMatchObject({ typedUsage: { totalTokens: 51 } });
    } finally {
      await NodeFSP.rm(dir, { recursive: true, force: true });
    }
  });

  it("gives anonymous parallel children distinct identities and closes missing final results", () => {
    const tracker = makePiSubagents();
    const events = tracker.observe(
      "call",
      snapshot(child({ session: undefined }), child({ session: undefined })),
    );
    expect(
      events.filter((event) => event.type === "task.started").map((event) => event.payload.taskId),
    ).toEqual(["pi:call:0", "pi:call:1"]);
    expect(
      tracker.finish("call", "failed", "Missing result").map((event) => event.payload.status),
    ).toEqual(["failed", "failed"]);
  });
});
