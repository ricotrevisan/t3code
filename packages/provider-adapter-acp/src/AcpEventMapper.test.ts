import {
  EventId,
  ProviderDriverKind,
  ProviderInstanceId,
  ThreadId,
  TurnId,
  type ProviderRuntimeEvent,
} from "@t3tools/contracts";
import type * as AcpSchema from "effect-acp/schema";
import { describe, expect, it } from "vite-plus/test";

import {
  finishAcpTurn,
  makeAcpEventMapperState,
  mapAcpSessionUpdate,
  type AcpEventMapperHost,
  type AcpEventMapperState,
} from "./AcpEventMapper.ts";

const sessionId = "session-1";

function makeHost(overrides: Partial<AcpEventMapperHost> = {}): AcpEventMapperHost {
  return {
    sessionId,
    provider: ProviderDriverKind.make("test-acp"),
    providerInstanceId: ProviderInstanceId.make("instance-1"),
    threadId: ThreadId.make("thread-1"),
    turnId: TurnId.make("turn-1"),
    stamp: (index) => ({
      eventId: EventId.make(`event-${index}`),
      createdAt: `2026-09-10T00:00:0${index}.000Z`,
    }),
    ...overrides,
  };
}

function map(
  state: AcpEventMapperState,
  notification: AcpSchema.SessionNotification,
  host: AcpEventMapperHost = makeHost(),
) {
  return mapAcpSessionUpdate({ state, notification, host });
}

function itemPayload(event: ProviderRuntimeEvent | undefined) {
  if (
    event?.type !== "item.started" &&
    event?.type !== "item.updated" &&
    event?.type !== "item.completed"
  ) {
    throw new Error("Expected an item lifecycle event");
  }
  return event.payload;
}

describe("AcpEventMapper", () => {
  it("orders an assistant item start before its content and reuses it for later chunks", () => {
    const firstNotification = {
      sessionId,
      update: {
        sessionUpdate: "agent_message_chunk",
        messageId: "message-1",
        content: { type: "text", text: "Hello" },
      },
    } satisfies AcpSchema.SessionNotification;
    const first = map(makeAcpEventMapperState(), firstNotification);

    expect(first.events.map((event) => event.type)).toEqual(["item.started", "content.delta"]);
    expect(first.events.map((event) => event.eventId)).toEqual(["event-0", "event-1"]);
    expect(first.events[0]?.itemId).toBe(first.events[1]?.itemId);
    expect(first.events[0]?.raw).toEqual({
      source: "acp.jsonrpc",
      method: "session/update",
      payload: firstNotification,
    });
    expect(itemPayload(first.events[0])).toMatchObject({
      itemType: "assistant_message",
      status: "inProgress",
    });
    expect(first.events[1]).toMatchObject({
      type: "content.delta",
      payload: { streamKind: "assistant_text", delta: "Hello" },
    });

    const second = map(first.state, {
      sessionId,
      update: {
        sessionUpdate: "agent_message_chunk",
        messageId: "message-1",
        content: { type: "text", text: " world" },
      },
    });
    expect(second.events.map((event) => event.type)).toEqual(["content.delta"]);
    expect(second.events[0]?.itemId).toBe(first.events[0]?.itemId);
  });

  it("completes the previous item before a different message starts", () => {
    const first = map(makeAcpEventMapperState(), {
      sessionId,
      update: {
        sessionUpdate: "agent_message_chunk",
        messageId: "message-1",
        content: { type: "text", text: "First" },
      },
    });
    const previousItemId = first.events[0]?.itemId;
    const nextNotification = {
      sessionId,
      update: {
        sessionUpdate: "agent_message_chunk",
        messageId: "message-2",
        content: { type: "text", text: "Second" },
      },
    } satisfies AcpSchema.SessionNotification;
    const next = map(first.state, nextNotification);

    expect(next.events.map((event) => event.type)).toEqual([
      "item.completed",
      "item.started",
      "content.delta",
    ]);
    expect(next.events.map((event) => event.eventId)).toEqual(["event-0", "event-1", "event-2"]);
    expect(next.events[0]?.itemId).toBe(previousItemId);
    expect(next.events[1]?.itemId).not.toBe(previousItemId);
    expect(next.events[1]?.itemId).toBe(next.events[2]?.itemId);
    expect(itemPayload(next.events[0])).toMatchObject({
      itemType: "assistant_message",
      status: "completed",
    });
    expect(next.events.every((event) => event.raw?.payload === nextNotification)).toBe(true);
  });

  it("preserves ordered chunks as deltas instead of treating them as replacements", () => {
    const first = map(makeAcpEventMapperState(), {
      sessionId,
      update: {
        sessionUpdate: "agent_message_chunk",
        content: { type: "text", text: "committed block one" },
      },
    });
    const second = map(first.state, {
      sessionId,
      update: {
        sessionUpdate: "agent_message_chunk",
        content: { type: "text", text: "committed block two" },
      },
    });

    expect(
      [...first.events, ...second.events]
        .filter((event) => event.type === "content.delta")
        .map((event) => event.payload.delta),
    ).toEqual(["committed block one", "committed block two"]);
  });

  it("merges partial tool updates and emits start, update, then completion", () => {
    const createdNotification = {
      sessionId,
      update: {
        sessionUpdate: "tool_call",
        toolCallId: "tool-1",
        title: "Run tests",
        kind: "execute",
        status: "pending",
        rawInput: { command: ["vp", "test"] },
      },
    } satisfies AcpSchema.SessionNotification;
    const created = map(makeAcpEventMapperState(), createdNotification);
    expect(created.events.map((event) => event.type)).toEqual(["item.started"]);
    expect(itemPayload(created.events[0])).toMatchObject({
      itemType: "command_execution",
      title: "Run tests",
      data: {
        toolCallId: "tool-1",
        kind: "execute",
        rawInput: { command: ["vp", "test"] },
      },
    });

    const progressed = map(created.state, {
      sessionId,
      update: {
        sessionUpdate: "tool_call_update",
        toolCallId: "tool-1",
        status: "in_progress",
      },
    });
    expect(progressed.events.map((event) => event.type)).toEqual(["item.updated"]);

    const completedNotification = {
      sessionId,
      update: {
        sessionUpdate: "tool_call_update",
        toolCallId: "tool-1",
        status: "completed",
        rawOutput: { exitCode: 0 },
      },
    } satisfies AcpSchema.SessionNotification;
    const completed = map(progressed.state, completedNotification);
    expect(completed.events.map((event) => event.type)).toEqual(["item.completed"]);
    expect(itemPayload(completed.events[0])).toMatchObject({
      itemType: "command_execution",
      status: "completed",
      title: "Run tests",
      data: {
        toolCallId: "tool-1",
        kind: "execute",
        status: "completed",
        rawInput: { command: ["vp", "test"] },
        rawOutput: { exitCode: 0 },
      },
    });
    expect(completed.state.toolCalls.get("tool-1")).toMatchObject({
      title: "Run tests",
      kind: "execute",
      status: "completed",
    });
    expect(completed.events[0]?.raw?.payload).toBe(completedNotification);
  });

  it("starts an unseen terminal tool before completing it", () => {
    const result = map(makeAcpEventMapperState(), {
      sessionId,
      update: {
        sessionUpdate: "tool_call",
        toolCallId: "tool-terminal",
        title: "Already done",
        kind: "other",
        status: "completed",
      },
    });

    expect(result.events.map((event) => event.type)).toEqual(["item.started", "item.completed"]);
    expect(result.events.map((event) => event.eventId)).toEqual(["event-0", "event-1"]);
  });

  it("maps generic tool kinds to dynamic tools without guessing from the title", () => {
    const result = map(makeAcpEventMapperState(), {
      sessionId,
      update: {
        sessionUpdate: "tool_call",
        toolCallId: "tool-other",
        title: "Terminal: vp test",
        kind: "other",
        status: "pending",
      },
    });

    expect(itemPayload(result.events[0])).toMatchObject({
      itemType: "dynamic_tool_call",
      title: "Terminal: vp test",
    });
  });

  it("maps thought text to a reasoning item and reasoning delta", () => {
    const notification = {
      sessionId,
      update: {
        sessionUpdate: "agent_thought_chunk",
        messageId: "thought-1",
        content: { type: "text", text: "Check the types" },
      },
    } satisfies AcpSchema.SessionNotification;
    const result = map(
      makeAcpEventMapperState(),
      notification,
      makeHost({
        source: "acp.cursor.extension",
        method: "session/update",
      }),
    );

    expect(result.events.map((event) => event.type)).toEqual(["item.started", "content.delta"]);
    expect(itemPayload(result.events[0])).toMatchObject({ itemType: "reasoning" });
    expect(result.events[1]).toMatchObject({
      type: "content.delta",
      payload: { streamKind: "reasoning_text", delta: "Check the types" },
      raw: { source: "acp.cursor.extension", method: "session/update", payload: notification },
    });
  });

  it("maps ACP context occupancy to canonical thread token usage", () => {
    const notification = {
      sessionId,
      update: {
        sessionUpdate: "usage_update",
        used: 123,
        size: 4096,
      },
    } satisfies AcpSchema.SessionNotification;
    const result = map(makeAcpEventMapperState(), notification);

    expect(result.events).toMatchObject([
      {
        type: "thread.token-usage.updated",
        payload: { usage: { usedTokens: 123, maxTokens: 4096 } },
        raw: { source: "acp.jsonrpc", method: "session/update", payload: notification },
      },
    ]);
  });

  it("stores config option updates, emits session.configured, and requests a snapshot refresh", () => {
    const configOptions = [
      {
        id: "model",
        name: "Model",
        category: "model",
        type: "select",
        currentValue: "fast",
        options: [{ value: "fast", name: "Fast" }],
      },
    ] satisfies ReadonlyArray<AcpSchema.SessionConfigOption>;
    const notification = {
      sessionId,
      update: { sessionUpdate: "config_option_update", configOptions },
    } satisfies AcpSchema.SessionNotification;
    const result = map(makeAcpEventMapperState(), notification);

    expect(result.state.configOptions).toBe(configOptions);
    expect(result.snapshotRefreshRequired).toBe(true);
    expect(result.events).toMatchObject([
      {
        type: "session.configured",
        payload: { config: { configOptions } },
        raw: { source: "acp.jsonrpc", method: "session/update", payload: notification },
      },
    ]);
  });

  it("finishes open assistant and reasoning items in deterministic order", () => {
    const assistant = map(makeAcpEventMapperState(), {
      sessionId,
      update: {
        sessionUpdate: "agent_message_chunk",
        content: { type: "text", text: "Answer" },
      },
    });
    const reasoning = map(assistant.state, {
      sessionId,
      update: {
        sessionUpdate: "agent_thought_chunk",
        content: { type: "text", text: "Reason" },
      },
    });
    const rawPayload = { sessionId, stopReason: "end_turn" };
    const finished = finishAcpTurn({
      state: reasoning.state,
      host: makeHost(),
      raw: {
        source: "acp.jsonrpc",
        method: "session/prompt",
        payload: rawPayload,
      },
    });

    expect(finished.events.map((event) => event.type)).toEqual([
      "item.completed",
      "item.completed",
    ]);
    expect(finished.events.map((event) => itemPayload(event).itemType)).toEqual([
      "assistant_message",
      "reasoning",
    ]);
    expect(finished.events.map((event) => event.eventId)).toEqual(["event-0", "event-1"]);
    expect(finished.events).toMatchObject([
      {
        raw: { source: "acp.jsonrpc", method: "session/prompt", payload: rawPayload },
      },
      {
        raw: { source: "acp.jsonrpc", method: "session/prompt", payload: rawPayload },
      },
    ]);
    expect(finished.state.assistantItem).toBeUndefined();
    expect(finished.state.reasoningItem).toBeUndefined();
  });

  it("ignores updates routed from a foreign ACP session without changing state", () => {
    const state = makeAcpEventMapperState();
    const result = map(
      state,
      {
        sessionId: "child-session",
        update: {
          sessionUpdate: "agent_message_chunk",
          content: { type: "text", text: "child output" },
        },
      },
      makeHost({
        stamp: () => {
          throw new Error("Foreign updates must not allocate event stamps");
        },
      }),
    );

    expect(result).toEqual({ state, events: [], snapshotRefreshRequired: false });
    expect(result.state).toBe(state);
  });
});
