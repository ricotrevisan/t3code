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

  it("emits terminal tool state once and maps late partial patches as updates", () => {
    const completed = map(makeAcpEventMapperState(), {
      sessionId,
      update: {
        sessionUpdate: "tool_call",
        toolCallId: "tool-late-patch",
        title: "Complete once",
        kind: "execute",
        status: "completed",
      },
    });
    expect(completed.events.map((event) => event.type)).toEqual(["item.started", "item.completed"]);
    expect(completed.state.toolCalls.get("tool-late-patch")?.terminalEmitted).toBe(true);

    const partialPatch = map(completed.state, {
      sessionId,
      update: {
        sessionUpdate: "tool_call_update",
        toolCallId: "tool-late-patch",
        rawOutput: { exitCode: 0 },
      },
    });
    expect(partialPatch.events.map((event) => event.type)).toEqual(["item.updated"]);
    expect(itemPayload(partialPatch.events[0])).toMatchObject({
      status: "completed",
      data: { status: "completed", rawOutput: { exitCode: 0 } },
    });

    const repeatedTerminalPatch = map(partialPatch.state, {
      sessionId,
      update: {
        sessionUpdate: "tool_call_update",
        toolCallId: "tool-late-patch",
        status: "completed",
      },
    });
    expect(repeatedTerminalPatch.events.map((event) => event.type)).toEqual(["item.updated"]);
  });

  it("drops retained tool payloads at turn finish and starts reused ids cleanly next turn", () => {
    const largeOutput = "x".repeat(1_000_000);
    const completed = map(makeAcpEventMapperState(), {
      sessionId,
      update: {
        sessionUpdate: "tool_call",
        toolCallId: "reused-tool",
        title: "Large result",
        kind: "execute",
        status: "completed",
        content: [{ type: "content", content: { type: "text", text: largeOutput } }],
        rawInput: { command: "large" },
        rawOutput: { stdout: largeOutput },
      },
    });
    expect(completed.state.toolCalls.get("reused-tool")?.rawOutput).toBeDefined();

    const finished = finishAcpTurn({
      state: completed.state,
      host: makeHost(),
      raw: { payload: { stopReason: "end_turn" } },
    });
    expect(finished.events).toEqual([]);
    expect(finished.state.toolCalls.size).toBe(0);

    const nextTurn = map(
      finished.state,
      {
        sessionId,
        update: {
          sessionUpdate: "tool_call",
          toolCallId: "reused-tool",
          title: "Fresh result",
          kind: "execute",
          status: "pending",
        },
      },
      makeHost({ turnId: TurnId.make("turn-2") }),
    );
    expect(nextTurn.events.map((event) => event.type)).toEqual(["item.started"]);
    expect(itemPayload(nextTurn.events[0])).toMatchObject({
      title: "Fresh result",
      data: { toolCallId: "reused-tool" },
    });
    expect(itemPayload(nextTurn.events[0]).data).not.toHaveProperty("rawInput");
    expect(itemPayload(nextTurn.events[0]).data).not.toHaveProperty("rawOutput");
    expect(itemPayload(nextTurn.events[0]).data).not.toHaveProperty("content");
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

  it("keeps image message chunks in the assistant item lifecycle with raw block data", () => {
    const content = {
      type: "image",
      data: "aW1hZ2U=",
      mimeType: "image/png",
      uri: "file:///tmp/image.png",
    } as const;
    const notification = {
      sessionId,
      update: {
        sessionUpdate: "agent_message_chunk",
        messageId: "image-message",
        content,
      },
    } satisfies AcpSchema.SessionNotification;
    const result = map(makeAcpEventMapperState(), notification);

    expect(result.events.map((event) => event.type)).toEqual([
      "item.started",
      "item.updated",
      "content.delta",
    ]);
    expect(result.events.every((event) => event.itemId === result.events[0]?.itemId)).toBe(true);
    expect(itemPayload(result.events[1])).toMatchObject({
      itemType: "assistant_message",
      status: "inProgress",
      data: { content },
    });
    expect(result.events[2]).toMatchObject({
      type: "content.delta",
      payload: {
        streamKind: "assistant_text",
        delta: "[Image: image/png; file:///tmp/image.png]",
      },
    });
    expect(JSON.stringify(result.events[2]?.payload)).not.toContain(content.data);
    expect(result.events.every((event) => event.raw?.payload === notification)).toBe(true);
  });

  it("keeps resource thought chunks in the reasoning item lifecycle with raw block data", () => {
    const content = {
      type: "resource_link",
      name: "design notes",
      uri: "file:///tmp/design.md",
      mimeType: "text/markdown",
    } as const;
    const result = map(makeAcpEventMapperState(), {
      sessionId,
      update: {
        sessionUpdate: "agent_thought_chunk",
        messageId: "resource-thought",
        content,
      },
    });

    expect(result.events.map((event) => event.type)).toEqual([
      "item.started",
      "item.updated",
      "content.delta",
    ]);
    expect(itemPayload(result.events[0])).toMatchObject({ itemType: "reasoning" });
    expect(itemPayload(result.events[1])).toMatchObject({
      itemType: "reasoning",
      data: { content },
    });
    expect(result.events[2]).toMatchObject({
      type: "content.delta",
      payload: {
        streamKind: "reasoning_text",
        delta: "[Resource: design notes; file:///tmp/design.md]",
      },
    });
  });

  it("renders embedded blob resources without exposing encoded binary", () => {
    const content = {
      type: "resource",
      resource: {
        blob: "JVBERi0xLjQKZW5jb2RlZC1wZGY=",
        mimeType: "application/pdf",
        uri: "file:///tmp/report.pdf",
      },
    } as const;
    const result = map(makeAcpEventMapperState(), {
      sessionId,
      update: {
        sessionUpdate: "agent_message_chunk",
        messageId: "blob-resource",
        content,
      },
    });

    expect(result.events.map((event) => event.type)).toEqual([
      "item.started",
      "item.updated",
      "content.delta",
    ]);
    expect(itemPayload(result.events[1])).toMatchObject({ data: { content } });
    expect(result.events[2]).toMatchObject({
      type: "content.delta",
      payload: {
        streamKind: "assistant_text",
        delta: "[Resource: application/pdf; file:///tmp/report.pdf]",
      },
    });
    expect(JSON.stringify(result.events[2]?.payload)).not.toContain(content.resource.blob);
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

  it("maps plans to canonical plan updates and retains the raw ACP plan", () => {
    const notification = {
      sessionId,
      update: {
        sessionUpdate: "plan",
        entries: [
          { content: "Inspect code", priority: "high", status: "in_progress" },
          { content: "Run tests", priority: "medium", status: "pending" },
          { content: "Ship", priority: "low", status: "completed" },
        ],
      },
    } satisfies AcpSchema.SessionNotification;
    const result = map(makeAcpEventMapperState(), notification);

    expect(result.events).toMatchObject([
      {
        type: "turn.plan.updated",
        payload: {
          plan: [
            { step: "Inspect code", status: "inProgress" },
            { step: "Run tests", status: "pending" },
            { step: "Ship", status: "completed" },
          ],
        },
        raw: { source: "acp.jsonrpc", method: "session/update", payload: notification },
      },
    ]);
  });

  it("stores available commands, emits session configuration, and refreshes snapshots", () => {
    const availableCommands = [
      { name: "review", description: "Review the current change" },
    ] satisfies ReadonlyArray<AcpSchema.AvailableCommand>;
    const notification = {
      sessionId,
      update: { sessionUpdate: "available_commands_update", availableCommands },
    } satisfies AcpSchema.SessionNotification;
    const result = map(makeAcpEventMapperState(), notification);

    expect(result.state.availableCommands).toBe(availableCommands);
    expect(result.snapshotRefreshRequired).toBe(true);
    expect(result.events).toMatchObject([
      {
        type: "session.configured",
        payload: { config: { availableCommands } },
        raw: { payload: notification },
      },
    ]);
  });

  it("stores the current mode, emits session configuration, and refreshes snapshots", () => {
    const notification = {
      sessionId,
      update: { sessionUpdate: "current_mode_update", currentModeId: "architect" },
    } satisfies AcpSchema.SessionNotification;
    const result = map(makeAcpEventMapperState(), notification);

    expect(result.state.currentModeId).toBe("architect");
    expect(result.snapshotRefreshRequired).toBe(true);
    expect(result.events).toMatchObject([
      {
        type: "session.configured",
        payload: { config: { currentModeId: "architect" } },
        raw: { payload: notification },
      },
    ]);
  });

  it("merges session info, emits thread metadata, and refreshes snapshots", () => {
    const titleNotification = {
      sessionId,
      update: {
        sessionUpdate: "session_info_update",
        title: "ACP session",
        updatedAt: "2026-09-10T00:00:00.000Z",
      },
    } satisfies AcpSchema.SessionNotification;
    const titled = map(makeAcpEventMapperState(), titleNotification);

    expect(titled.state.sessionInfo).toEqual({
      title: "ACP session",
      updatedAt: "2026-09-10T00:00:00.000Z",
    });
    expect(titled.snapshotRefreshRequired).toBe(true);
    expect(titled.events).toMatchObject([
      {
        type: "thread.metadata.updated",
        payload: {
          name: "ACP session",
          metadata: {
            sessionId,
            title: "ACP session",
            updatedAt: "2026-09-10T00:00:00.000Z",
          },
        },
        raw: { payload: titleNotification },
      },
    ]);

    const cleared = map(titled.state, {
      sessionId,
      update: { sessionUpdate: "session_info_update", title: null },
    });
    expect(cleared.state.sessionInfo).toEqual({
      title: null,
      updatedAt: "2026-09-10T00:00:00.000Z",
    });
    expect(cleared.events[0]).toMatchObject({
      type: "thread.metadata.updated",
      payload: { metadata: { sessionId, title: null } },
    });
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

  it("intentionally suppresses echoed user message chunks", () => {
    const state = makeAcpEventMapperState();
    const result = map(state, {
      sessionId,
      update: {
        sessionUpdate: "user_message_chunk",
        content: { type: "text", text: "Already persisted prompt" },
      },
    });

    expect(result).toEqual({ state, events: [], snapshotRefreshRequired: false });
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
