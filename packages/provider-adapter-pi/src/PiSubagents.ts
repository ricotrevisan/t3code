import {
  RuntimeTaskId,
  type ProviderRuntimeEvent,
  type RuntimeTaskUsage,
} from "@t3tools/contracts";
import * as Exit from "effect/Exit";
import * as Schema from "effect/Schema";

const Result = Schema.Struct({
  callIndex: Schema.optional(Schema.Int),
  agent: Schema.String,
  prompt: Schema.optional(Schema.String),
  session: Schema.optional(
    Schema.Struct({ id: Schema.String, handle: Schema.optional(Schema.String) }),
  ),
  exitCode: Schema.Int,
  model: Schema.optional(Schema.String),
  stopReason: Schema.optional(Schema.String),
  errorMessage: Schema.optional(Schema.String),
  usage: Schema.optional(
    Schema.Struct({
      input: Schema.Finite,
      output: Schema.Finite,
      cacheRead: Schema.Finite,
      cacheWrite: Schema.Finite,
    }),
  ),
  messages: Schema.optional(
    Schema.Array(
      Schema.Struct({
        role: Schema.String,
        content: Schema.optional(Schema.Unknown),
      }),
    ),
  ),
});
const decodeDetails = Schema.decodeUnknownExit(
  Schema.Struct({
    details: Schema.Struct({
      kind: Schema.Literal("pi-subagent"),
      results: Schema.Array(Schema.Unknown),
    }),
  }),
);
const decodeResult = Schema.decodeUnknownExit(Result);
const decodeParts = Schema.decodeUnknownExit(
  Schema.Array(
    Schema.Struct({
      type: Schema.String,
      text: Schema.optional(Schema.String),
      name: Schema.optional(Schema.String),
    }),
  ),
);
type TaskEvent = Extract<
  ProviderRuntimeEvent,
  { type: "task.started" | "task.progress" | "task.completed" }
>;
type Event = TaskEvent extends infer E
  ? E extends TaskEvent
    ? Pick<E, "type" | "payload">
    : never
  : never;
type Linkage = Extract<TaskEvent, { type: "task.started" }>["payload"];
const bounded = (text: string | undefined) => text?.trim().slice(0, 500) || undefined;
const count = (n: number) => (Number.isSafeInteger(n) && n >= 0 ? n : 0);
const zero = (): RuntimeTaskUsage => ({
  totalTokens: 0,
  inputTokens: 0,
  outputTokens: 0,
  cachedInputTokens: 0,
});
const add = (a: RuntimeTaskUsage, b: RuntimeTaskUsage): RuntimeTaskUsage => ({
  totalTokens: a.totalTokens + b.totalTokens,
  inputTokens: (a.inputTokens ?? 0) + (b.inputTokens ?? 0),
  outputTokens: (a.outputTokens ?? 0) + (b.outputTokens ?? 0),
  cachedInputTokens: (a.cachedInputTokens ?? 0) + (b.cachedInputTokens ?? 0),
});

/** Converts @mjakl/pi-subagent snapshots before generic tool-output truncation.
 * Keeps session identity across activations, with cumulative usage for the client fold. */
export function makePiSubagents() {
  const totals = new Map<string, RuntimeTaskUsage>();
  const calls = new Map<
    string,
    Map<
      number,
      {
        linkage: Linkage;
        base: RuntimeTaskUsage;
        usage: RuntimeTaskUsage;
        terminal: boolean;
        progress?: string;
      }
    >
  >();

  function observe(toolCallId: string, value: unknown): Event[] {
    const decoded = decodeDetails(value);
    if (Exit.isFailure(decoded)) return [];
    const events: Event[] = [];
    let batch = calls.get(toolCallId);
    if (!batch) {
      batch = new Map();
      calls.set(toolCallId, batch);
    }
    for (const [index, raw] of decoded.value.details.results.entries()) {
      const decodedResult = decodeResult(raw);
      if (Exit.isFailure(decodedResult)) continue;
      const result = decodedResult.value;
      const key = result.callIndex ?? index;
      let child = batch.get(key);
      const firstObservation = child === undefined;
      if (child?.terminal) continue;
      if (!child) {
        const id = RuntimeTaskId.make(`pi:${result.session?.id.trim() || `${toolCallId}:${key}`}`);
        const title = bounded(result.session?.handle) ?? bounded(result.agent) ?? "Subagent";
        const linkage: Linkage = {
          taskId: id,
          taskType: "subagent",
          title,
          ...(bounded(result.agent) ? { role: bounded(result.agent)! } : {}),
          ...(bounded(result.model) ? { model: bounded(result.model)! } : {}),
          toolUseId: toolCallId,
        };
        child = { linkage, base: totals.get(id) ?? zero(), usage: zero(), terminal: false };
        batch.set(key, child);
        events.push({
          type: "task.started",
          payload: { ...linkage, description: bounded(result.prompt) ?? title },
        });
      }
      if (bounded(result.model))
        child.linkage = { ...child.linkage, model: bounded(result.model)! };
      if (result.usage) {
        const u = result.usage;
        child.usage = {
          totalTokens: count(u.input) + count(u.output) + count(u.cacheRead) + count(u.cacheWrite),
          inputTokens: count(u.input) + count(u.cacheWrite),
          outputTokens: count(u.output),
          cachedInputTokens: count(u.cacheRead),
        };
      }
      const typedUsage = add(child.base, child.usage);
      let summary: string | undefined;
      let lastToolName: string | undefined;
      for (const message of result.messages ?? []) {
        if (message.role !== "assistant") continue;
        const parts = decodeParts(message.content);
        if (Exit.isFailure(parts)) continue;
        for (const part of parts.value) {
          if (part.type === "text" && bounded(part.text)) summary = bounded(part.text);
          if (part.type === "toolCall" && bounded(part.name)) lastToolName = bounded(part.name);
        }
      }
      if (firstObservation && result.exitCode !== -1) {
        events.push({
          type: "task.progress",
          payload: {
            ...child.linkage,
            description: bounded(result.prompt) ?? child.linkage.title!,
            status: "running",
          },
        });
      }
      if (result.exitCode !== -1) {
        child.terminal = true;
        totals.set(child.linkage.taskId, typedUsage);
        const status =
          result.stopReason === "aborted"
            ? "stopped"
            : result.exitCode === 0
              ? "completed"
              : "failed";
        const detail = bounded(result.errorMessage) ?? summary;
        events.push({
          type: "task.completed",
          payload: {
            ...child.linkage,
            status,
            typedUsage,
            ...(detail ? { summary: detail } : {}),
          },
        });
      } else {
        // Tool changes and usage snapshots provide useful progress without persisting every text delta.
        const payload = {
          ...child.linkage,
          description: bounded(result.prompt) ?? child.linkage.title!,
          typedUsage,
          ...(lastToolName ? { lastToolName } : {}),
          status: "running" as const,
        };
        const fingerprint = JSON.stringify(payload);
        if (fingerprint !== child.progress) {
          child.progress = fingerprint;
          events.push({ type: "task.progress", payload });
        }
      }
    }
    return events;
  }

  function finish(
    toolCallId: string,
    status: "failed" | "stopped",
    summary: string,
  ): Extract<Event, { type: "task.completed" }>[] {
    const batch = calls.get(toolCallId);
    calls.delete(toolCallId);
    const events: Extract<Event, { type: "task.completed" }>[] = [];
    for (const child of batch?.values() ?? []) {
      if (child.terminal) continue;
      const typedUsage = add(child.base, child.usage);
      totals.set(child.linkage.taskId, typedUsage);
      events.push({
        type: "task.completed",
        payload: { ...child.linkage, status, summary, typedUsage },
      });
    }
    return events;
  }

  return {
    observe,
    finish,
    interrupt: () =>
      [...calls.keys()].flatMap((id) => finish(id, "stopped", "Subagent interrupted")),
  };
}
