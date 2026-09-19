import {
  RuntimeItemId,
  type EventId,
  type ProviderAdapterPackageReference,
  type ProviderDriverKind,
  type ProviderInstanceId,
  type ProviderRuntimeEvent,
  type RuntimeEventRawSource,
  type ThreadId,
  type TurnId,
  type ToolLifecycleItemType,
} from "@t3tools/contracts";
import type * as AcpSchema from "effect-acp/schema";

type AcpUpdate = AcpSchema.SessionNotification["update"];
type AcpToolUpdate = Extract<
  AcpUpdate,
  { readonly sessionUpdate: "tool_call" | "tool_call_update" }
>;
type AcpTextUpdate = Extract<
  AcpUpdate,
  { readonly sessionUpdate: "agent_message_chunk" | "agent_thought_chunk" }
>;
type AcpRawSource = Extract<RuntimeEventRawSource, "acp.jsonrpc" | `acp.${string}.extension`>;

type AcpToolKind = NonNullable<AcpToolUpdate["kind"]>;
type AcpToolStatus = NonNullable<AcpToolUpdate["status"]>;

export interface AcpEventStamp {
  readonly eventId: EventId;
  readonly createdAt: string;
}

export interface AcpMappedToolCall {
  readonly toolCallId: string;
  readonly title: string | null;
  readonly kind: AcpToolKind | null;
  readonly status: AcpToolStatus | null;
  readonly content: ReadonlyArray<AcpSchema.ToolCallContent> | null;
  readonly locations: ReadonlyArray<AcpSchema.ToolCallLocation> | null;
  readonly terminalEmitted: boolean;
  readonly rawInput?: unknown;
  readonly rawOutput?: unknown;
}

export interface AcpSessionInfoState {
  readonly title?: string | null;
  readonly updatedAt?: string | null;
}

interface AcpContentItemState {
  readonly itemId: string;
  readonly messageId?: string;
  readonly turnId?: TurnId;
}

export interface AcpEventMapperState {
  readonly toolCalls: ReadonlyMap<string, AcpMappedToolCall>;
  readonly configOptions: ReadonlyArray<AcpSchema.SessionConfigOption> | undefined;
  readonly availableCommands: ReadonlyArray<AcpSchema.AvailableCommand> | undefined;
  readonly currentModeId: string | undefined;
  readonly sessionInfo: AcpSessionInfoState | undefined;
  readonly assistantItem: AcpContentItemState | undefined;
  readonly reasoningItem: AcpContentItemState | undefined;
  readonly nextContentItemIndex: number;
}

export interface AcpEventMapperHost {
  /** The root ACP session owned by this mapper. Updates for all other sessions are ignored. */
  readonly sessionId: string;
  readonly provider: ProviderDriverKind;
  readonly providerInstanceId?: ProviderInstanceId;
  readonly adapterPackage?: ProviderAdapterPackageReference | null;
  readonly threadId: ThreadId;
  readonly turnId?: TurnId;
  readonly source?: AcpRawSource;
  readonly method?: string;
  /** Called in output order. The callback must return a fresh identity for each index. */
  readonly stamp: (eventIndex: number) => AcpEventStamp;
}

export interface AcpEventMapperRaw {
  readonly source?: AcpRawSource;
  readonly method?: string;
  readonly payload: unknown;
}

export interface AcpEventMapperResult {
  readonly state: AcpEventMapperState;
  readonly events: ReadonlyArray<ProviderRuntimeEvent>;
  /** Config updates change the adapter snapshot even when the canonical event is not consumed. */
  readonly snapshotRefreshRequired: boolean;
}

export function makeAcpEventMapperState(
  configOptions?: ReadonlyArray<AcpSchema.SessionConfigOption>,
): AcpEventMapperState {
  return {
    toolCalls: new Map(),
    configOptions,
    availableCommands: undefined,
    currentModeId: undefined,
    sessionInfo: undefined,
    assistantItem: undefined,
    reasoningItem: undefined,
    nextContentItemIndex: 0,
  };
}

const hasOwn = (value: object, key: PropertyKey): boolean =>
  Object.prototype.hasOwnProperty.call(value, key);

function itemTypeFromToolKind(kind: AcpToolKind | null): ToolLifecycleItemType {
  switch (kind) {
    case "execute":
      return "command_execution";
    case "edit":
    case "delete":
    case "move":
      return "file_change";
    case "search":
    case "fetch":
      return "web_search";
    default:
      return "dynamic_tool_call";
  }
}

function statusFromToolStatus(
  status: AcpToolStatus | null,
): "inProgress" | "completed" | "failed" | undefined {
  switch (status) {
    case "pending":
    case "in_progress":
      return "inProgress";
    case "completed":
      return "completed";
    case "failed":
      return "failed";
    default:
      return undefined;
  }
}

function mergeToolCall(
  previous: AcpMappedToolCall | undefined,
  update: AcpToolUpdate,
): AcpMappedToolCall {
  const creation = update.sessionUpdate === "tool_call";
  return {
    toolCallId: update.toolCallId,
    title: creation || hasOwn(update, "title") ? (update.title ?? null) : (previous?.title ?? null),
    kind: creation || hasOwn(update, "kind") ? (update.kind ?? null) : (previous?.kind ?? null),
    status:
      creation || hasOwn(update, "status")
        ? (update.status ?? (creation ? "pending" : null))
        : (previous?.status ?? null),
    content:
      creation || hasOwn(update, "content")
        ? (update.content ?? null)
        : (previous?.content ?? null),
    locations:
      creation || hasOwn(update, "locations")
        ? (update.locations ?? null)
        : (previous?.locations ?? null),
    terminalEmitted: previous?.terminalEmitted ?? false,
    ...(hasOwn(update, "rawInput")
      ? { rawInput: update.rawInput }
      : previous && hasOwn(previous, "rawInput")
        ? { rawInput: previous.rawInput }
        : {}),
    ...(hasOwn(update, "rawOutput")
      ? { rawOutput: update.rawOutput }
      : previous && hasOwn(previous, "rawOutput")
        ? { rawOutput: previous.rawOutput }
        : {}),
  };
}

function toolData(toolCall: AcpMappedToolCall): Record<string, unknown> {
  return {
    toolCallId: toolCall.toolCallId,
    ...(toolCall.kind !== null ? { kind: toolCall.kind } : {}),
    ...(toolCall.status !== null ? { status: toolCall.status } : {}),
    ...(toolCall.content !== null ? { content: toolCall.content } : {}),
    ...(toolCall.locations !== null ? { locations: toolCall.locations } : {}),
    ...(hasOwn(toolCall, "rawInput") ? { rawInput: toolCall.rawInput } : {}),
    ...(hasOwn(toolCall, "rawOutput") ? { rawOutput: toolCall.rawOutput } : {}),
  };
}

function makeBase(host: AcpEventMapperHost, index: number) {
  return {
    ...host.stamp(index),
    provider: host.provider,
    ...(host.providerInstanceId !== undefined
      ? { providerInstanceId: host.providerInstanceId }
      : {}),
    ...(host.adapterPackage !== undefined ? { adapterPackage: host.adapterPackage } : {}),
    threadId: host.threadId,
    ...(host.turnId !== undefined ? { turnId: host.turnId } : {}),
  };
}

function makeRaw(
  host: AcpEventMapperHost,
  payload: unknown,
  overrides?: Pick<AcpEventMapperRaw, "source" | "method">,
) {
  return {
    source: overrides?.source ?? host.source ?? ("acp.jsonrpc" as const),
    method: overrides?.method ?? host.method ?? "session/update",
    payload,
  };
}

function makeContentCompletedEvent(
  host: AcpEventMapperHost,
  item: AcpContentItemState,
  itemType: "assistant_message" | "reasoning",
  index: number,
  raw: ReturnType<typeof makeRaw>,
): ProviderRuntimeEvent {
  return {
    type: "item.completed",
    ...makeBase(host, index),
    ...(item.turnId !== undefined ? { turnId: item.turnId } : {}),
    itemId: RuntimeItemId.make(item.itemId),
    payload: { itemType, status: "completed" },
    raw,
  };
}

function makeToolEvent(
  host: AcpEventMapperHost,
  notification: AcpSchema.SessionNotification,
  toolCall: AcpMappedToolCall,
  type: "item.started" | "item.updated" | "item.completed",
  index: number,
): ProviderRuntimeEvent {
  const title = toolCall.title?.trim();
  const mappedStatus = statusFromToolStatus(toolCall.status);
  return {
    type,
    ...makeBase(host, index),
    itemId: RuntimeItemId.make(toolCall.toolCallId),
    payload: {
      itemType: itemTypeFromToolKind(toolCall.kind),
      ...(type === "item.started"
        ? { status: "inProgress" as const }
        : mappedStatus !== undefined
          ? { status: mappedStatus }
          : {}),
      ...(title ? { title } : {}),
      data: toolData(toolCall),
    },
    raw: makeRaw(host, notification),
  };
}

function normalizedMessageId(update: AcpTextUpdate): string | undefined {
  const value = update.messageId?.trim();
  return value ? value : undefined;
}

function contentItemMatches(
  current: AcpContentItemState | undefined,
  messageId: string | undefined,
  turnId: TurnId | undefined,
): boolean {
  if (!current || current.turnId !== turnId) {
    return false;
  }
  return (
    messageId === undefined || current.messageId === undefined || current.messageId === messageId
  );
}

function makeContentItemId(
  kind: "assistant" | "reasoning",
  host: AcpEventMapperHost,
  messageId: string | undefined,
  index: number,
): string {
  const scope = host.turnId ?? host.threadId;
  const suffix = messageId ?? String(index);
  return `acp:${kind}:${host.sessionId}:${scope}:${suffix}`;
}

function nonTextContentFallback(
  content: Exclude<AcpSchema.ContentBlock, { readonly type: "text" }>,
): string {
  switch (content.type) {
    case "image":
      return `[Image: ${content.mimeType}${content.uri ? `; ${content.uri}` : ""}]`;
    case "audio":
      return `[Audio: ${content.mimeType}]`;
    case "resource_link": {
      const label = content.title?.trim() || content.name.trim() || "resource";
      return `[Resource: ${label}${content.uri ? `; ${content.uri}` : ""}]`;
    }
    case "resource":
      if ("text" in content.resource && content.resource.text) {
        return content.resource.text;
      }
      return `[Resource: ${content.resource.mimeType || "binary"}${
        content.resource.uri ? `; ${content.resource.uri}` : ""
      }]`;
  }
}

function mapContentUpdate(
  state: AcpEventMapperState,
  notification: AcpSchema.SessionNotification,
  host: AcpEventMapperHost,
  update: AcpTextUpdate,
): AcpEventMapperResult {
  const kind = update.sessionUpdate === "agent_message_chunk" ? "assistant" : "reasoning";
  const current = kind === "assistant" ? state.assistantItem : state.reasoningItem;
  const messageId = normalizedMessageId(update);
  const needsStart = !contentItemMatches(current, messageId, host.turnId);
  const completesCurrent =
    current !== undefined &&
    current.turnId === host.turnId &&
    current.messageId !== undefined &&
    messageId !== undefined &&
    current.messageId !== messageId;
  const itemId =
    !needsStart && current
      ? current.itemId
      : makeContentItemId(kind, host, messageId, state.nextContentItemIndex);
  const itemState: AcpContentItemState = {
    itemId,
    ...(messageId !== undefined
      ? { messageId }
      : current?.messageId !== undefined
        ? { messageId: current.messageId }
        : {}),
    ...(host.turnId !== undefined ? { turnId: host.turnId } : {}),
  };
  const nextState: AcpEventMapperState = {
    ...state,
    ...(kind === "assistant" ? { assistantItem: itemState } : { reasoningItem: itemState }),
    nextContentItemIndex: state.nextContentItemIndex + (needsStart ? 1 : 0),
  };
  const raw = makeRaw(host, notification);
  const events: Array<ProviderRuntimeEvent> = [];
  if (completesCurrent && current !== undefined) {
    events.push(
      makeContentCompletedEvent(
        host,
        current,
        kind === "assistant" ? "assistant_message" : "reasoning",
        events.length,
        raw,
      ),
    );
  }
  if (needsStart) {
    events.push({
      type: "item.started",
      ...makeBase(host, events.length),
      itemId: RuntimeItemId.make(itemId),
      payload: {
        itemType: kind === "assistant" ? "assistant_message" : "reasoning",
        status: "inProgress",
      },
      raw,
    });
  }
  if (update.content.type === "text") {
    events.push({
      type: "content.delta",
      ...makeBase(host, events.length),
      itemId: RuntimeItemId.make(itemId),
      payload: {
        streamKind: kind === "assistant" ? "assistant_text" : "reasoning_text",
        delta: update.content.text,
      },
      raw,
    });
  } else {
    events.push({
      type: "item.updated",
      ...makeBase(host, events.length),
      itemId: RuntimeItemId.make(itemId),
      payload: {
        itemType: kind === "assistant" ? "assistant_message" : "reasoning",
        status: "inProgress",
        data: { content: update.content },
      },
      raw,
    });
    events.push({
      type: "content.delta",
      ...makeBase(host, events.length),
      itemId: RuntimeItemId.make(itemId),
      payload: {
        streamKind: kind === "assistant" ? "assistant_text" : "reasoning_text",
        delta: nonTextContentFallback(update.content),
      },
      raw,
    });
  }
  return { state: nextState, events, snapshotRefreshRequired: false };
}

/** Completes open content items before the host emits the canonical turn completion. */
export function finishAcpTurn(input: {
  readonly state: AcpEventMapperState;
  readonly host: AcpEventMapperHost;
  readonly raw: AcpEventMapperRaw;
}): AcpEventMapperResult {
  const { state, host } = input;
  const raw = makeRaw(host, input.raw.payload, input.raw);
  const events: Array<ProviderRuntimeEvent> = [];
  if (state.assistantItem !== undefined) {
    events.push(
      makeContentCompletedEvent(host, state.assistantItem, "assistant_message", events.length, raw),
    );
  }
  if (state.reasoningItem !== undefined) {
    events.push(
      makeContentCompletedEvent(host, state.reasoningItem, "reasoning", events.length, raw),
    );
  }

  return {
    state: {
      ...state,
      toolCalls: new Map(),
      assistantItem: undefined,
      reasoningItem: undefined,
    },
    events,
    snapshotRefreshRequired: false,
  };
}

function planStatus(status: AcpSchema.PlanEntry["status"]): "pending" | "inProgress" | "completed" {
  switch (status) {
    case "pending":
      return "pending";
    case "in_progress":
      return "inProgress";
    case "completed":
      return "completed";
  }
}

function assertNever(value: never): never {
  throw new Error(`Unsupported ACP session update: ${JSON.stringify(value)}`);
}

/**
 * Maps one standard ACP v1 `session/update` notification without performing I/O.
 * State and events are returned together so callers can serialize updates per ACP session.
 */
export function mapAcpSessionUpdate(input: {
  readonly state: AcpEventMapperState;
  readonly notification: AcpSchema.SessionNotification;
  readonly host: AcpEventMapperHost;
}): AcpEventMapperResult {
  const { state, notification, host } = input;
  if (notification.sessionId !== host.sessionId) {
    return { state, events: [], snapshotRefreshRequired: false };
  }

  const update = notification.update;
  switch (update.sessionUpdate) {
    // User messages already enter the canonical timeline through the prompt command.
    case "user_message_chunk":
      return { state, events: [], snapshotRefreshRequired: false };

    case "agent_message_chunk":
    case "agent_thought_chunk":
      return mapContentUpdate(state, notification, host, update);

    case "tool_call":
    case "tool_call_update": {
      const toolCallId = update.toolCallId.trim();
      if (!toolCallId) {
        return { state, events: [], snapshotRefreshRequired: false };
      }
      const previous = state.toolCalls.get(toolCallId);
      const merged = mergeToolCall(previous, { ...update, toolCallId });
      const terminal = merged.status === "completed" || merged.status === "failed";
      const emitsTerminal = terminal && !merged.terminalEmitted;
      const retained = emitsTerminal ? { ...merged, terminalEmitted: true } : merged;
      const toolCalls = new Map(state.toolCalls);
      toolCalls.set(toolCallId, retained);
      const nextState = { ...state, toolCalls };
      const events: Array<ProviderRuntimeEvent> = [];
      if (previous === undefined) {
        events.push(makeToolEvent(host, notification, retained, "item.started", events.length));
      }
      if (previous !== undefined || terminal) {
        events.push(
          makeToolEvent(
            host,
            notification,
            retained,
            emitsTerminal ? "item.completed" : "item.updated",
            events.length,
          ),
        );
      }
      return { state: nextState, events, snapshotRefreshRequired: false };
    }

    case "plan":
      return {
        state,
        events: [
          {
            type: "turn.plan.updated",
            ...makeBase(host, 0),
            payload: {
              plan: update.entries.flatMap((entry) => {
                const step = entry.content.trim();
                return step ? [{ step, status: planStatus(entry.status) }] : [];
              }),
            },
            raw: makeRaw(host, notification),
          },
        ],
        snapshotRefreshRequired: false,
      };

    case "available_commands_update": {
      const nextState = { ...state, availableCommands: update.availableCommands };
      return {
        state: nextState,
        events: [
          {
            type: "session.configured",
            ...makeBase(host, 0),
            payload: { config: { availableCommands: update.availableCommands } },
            raw: makeRaw(host, notification),
          },
        ],
        snapshotRefreshRequired: true,
      };
    }

    case "current_mode_update": {
      const nextState = { ...state, currentModeId: update.currentModeId };
      return {
        state: nextState,
        events: [
          {
            type: "session.configured",
            ...makeBase(host, 0),
            payload: { config: { currentModeId: update.currentModeId } },
            raw: makeRaw(host, notification),
          },
        ],
        snapshotRefreshRequired: true,
      };
    }

    case "config_option_update": {
      const nextState = { ...state, configOptions: update.configOptions };
      return {
        state: nextState,
        events: [
          {
            type: "session.configured",
            ...makeBase(host, 0),
            payload: { config: { configOptions: update.configOptions } },
            raw: makeRaw(host, notification),
          },
        ],
        snapshotRefreshRequired: true,
      };
    }

    case "session_info_update": {
      const sessionInfo: AcpSessionInfoState = {
        ...state.sessionInfo,
        ...(hasOwn(update, "title") ? { title: update.title ?? null } : {}),
        ...(hasOwn(update, "updatedAt") ? { updatedAt: update.updatedAt ?? null } : {}),
      };
      const name = sessionInfo.title?.trim();
      const nextState = { ...state, sessionInfo };
      return {
        state: nextState,
        events: [
          {
            type: "thread.metadata.updated",
            ...makeBase(host, 0),
            payload: {
              ...(name ? { name } : {}),
              metadata: { sessionId: host.sessionId, ...sessionInfo },
            },
            raw: makeRaw(host, notification),
          },
        ],
        snapshotRefreshRequired: true,
      };
    }

    case "usage_update":
      return {
        state,
        events: [
          {
            type: "thread.token-usage.updated",
            ...makeBase(host, 0),
            payload: {
              usage: {
                usedTokens: update.used,
                ...(update.size > 0 ? { maxTokens: update.size } : {}),
              },
            },
            raw: makeRaw(host, notification),
          },
        ],
        snapshotRefreshRequired: false,
      };
  }

  return assertNever(update);
}
