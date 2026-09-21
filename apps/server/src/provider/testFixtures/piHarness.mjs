import * as NodeFS from "node:fs";
import * as NodePath from "node:path";

/**
 * Fake Pi RPC harness used by the pi-rpc adapter conformance fixtures.
 *
 * Speaks the Pi RPC protocol subset over stdio JSONL (strict LF framing):
 * commands `switch_session`, `get_state`, `get_available_models`, `get_available_thinking_levels`,
 * `set_model`, `set_thinking_level`, `get_session_stats`, `get_entries`,
 * `prompt`, `steer`, `follow_up`, `abort`; plus the extension-UI
 * request/response sub-protocol. See the vendored Pi RPC doc for the real
 * protocol: https://github.com/earendil-works/pi (packages/coding-agent/docs/rpc.md).
 *
 * Test handles in `prompt.message`:
 * - `!tool ...`  -> runs a bash tool call before finishing.
 * - `!hold ...`  -> streams one delta then waits for `steer`/`abort`.
 * - `?select:...`-> opens an extension-UI select and waits for the response.
 */
const sessionDirectoryFlag = process.argv.indexOf("--session-dir");
const sessionDirectory =
  sessionDirectoryFlag >= 0 && process.argv[sessionDirectoryFlag + 1]
    ? process.argv[sessionDirectoryFlag + 1]
    : "/tmp/pi-sessions";
NodeFS.mkdirSync(sessionDirectory, { recursive: true });
const initialSessionFile = NodePath.join(sessionDirectory, "demo.jsonl");
NodeFS.closeSync(NodeFS.openSync(initialSessionFile, "a"));

const state = {
  sessionFile: initialSessionFile,
  sessionId: "pi-session-1",
  sessionName: undefined,
  model: {
    provider: "anthropic",
    id: "claude-sonnet-4",
    name: "Claude Sonnet 4",
    contextWindow: 200000,
  },
  thinkingLevel: "medium",
  entries: [],
  held: null,
  leafId: null,
};

const models = [
  state.model,
  { provider: "openai", id: "gpt-5.6", name: "GPT-5.6", contextWindow: 400000 },
  { provider: "custom", id: "gpt-5.6", name: "Custom GPT-5.6", contextWindow: 400000 },
];

const respond = (id, command, success, data, error) => {
  process.stdout.write(
    `${JSON.stringify({
      type: "response",
      ...(id === undefined ? {} : { id }),
      command,
      success,
      ...(data === undefined ? {} : { data }),
      ...(error === undefined ? {} : { error }),
    })}\n`,
  );
};

const emit = (event) => {
  process.stdout.write(`${JSON.stringify(event)}\n`);
};

const usage = () => ({
  input: 120,
  output: 30,
  cacheRead: 0,
  cacheWrite: 0,
  totalTokens: 150,
  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
});

const pushEntry = (message) => {
  const id = `entry-${state.entries.length + 1}`;
  state.entries.push({ type: "message", id, parentId: state.leafId, message });
  state.leafId = id;
  NodeFS.writeFileSync(
    state.sessionFile,
    state.entries.map((entry) => JSON.stringify(entry)).join("\n"),
  );
  return id;
};

const pushMetadata = (type) => {
  const id = `entry-${state.entries.length + 1}`;
  state.entries.push({ type, id, parentId: state.leafId });
  state.leafId = id;
};

const finishMessage = (reply, stopReason = "stop") => {
  const message = {
    role: "assistant",
    content: [{ type: "text", text: reply }],
    usage: usage(),
    stopReason,
    ...(stopReason === "error" ? { errorMessage: "Inference failed" } : {}),
  };
  pushEntry(message);
  emit({ type: "message_end", message });
  emit({ type: "turn_end", message, toolResults: [] });
  return message;
};

const settle = (message) => {
  state.held = null;
  emit({ type: "agent_end", messages: [message], willRetry: false });
  emit({ type: "agent_settled" });
};

const runTurn = (message, reply) => {
  emit({ type: "agent_start" });
  emit({ type: "turn_start" });
  if (message.startsWith("!subagents")) {
    const toolCallId = `delegation-${state.entries.length}`;
    const toolName = "subagent";
    const results = ["baseline", "audit"].map((handle, callIndex) => ({
      callIndex,
      agent: "worker",
      prompt: `Review ${handle}`,
      session: { id: `child-${handle}`, handle },
      exitCode: -1,
      model: "gpt-6-astra",
      messages: [],
      usage: { input: 10, output: 5, cacheRead: 3, cacheWrite: 2 },
    }));
    const partialResult = () => ({ details: { kind: "pi-subagent", results } });
    emit({ type: "tool_execution_start", toolCallId, toolName, args: { calls: [] } });
    emit({ type: "tool_execution_update", toolCallId, toolName, partialResult: partialResult() });
    emit({ type: "tool_execution_update", toolCallId, toolName, partialResult: partialResult() });
    results[0].exitCode = 0;
    results[0].messages = [
      { role: "assistant", content: [{ type: "text", text: "Baseline checked" }] },
    ];
    results[1].exitCode = 1;
    results[1].errorMessage = "Audit failed";
    pushEntry({ role: "toolResult", toolCallId, toolName, ...partialResult() });
    emit({
      type: "tool_execution_end",
      toolCallId,
      toolName,
      result: partialResult(),
      isError: true,
    });
  }
  if (message.startsWith("!tool")) {
    emit({
      type: "message_update",
      usage: { ...usage(), output: 0, totalTokens: 120 },
      assistantMessageEvent: {
        type: "toolcall_start",
        contentIndex: 0,
        id: "call_1",
        toolName: "bash",
      },
    });
    emit({
      type: "tool_execution_start",
      toolCallId: "call_1",
      toolName: "bash",
      args: { command: "ls" },
    });
    emit({
      type: "tool_execution_update",
      toolCallId: "call_1",
      toolName: "bash",
      args: { command: "ls" },
      partialResult: { content: [{ type: "text", text: "file" }] },
    });
    emit({
      type: "tool_execution_end",
      toolCallId: "call_1",
      toolName: "bash",
      result: { content: [{ type: "text", text: "file.txt" }] },
      isError: false,
    });
    finishMessage("", "toolUse");
    emit({ type: "turn_start" });
  }
  if (message.startsWith("!retry")) {
    const failed = finishMessage("", "error");
    emit({ type: "agent_end", messages: [failed], willRetry: true });
    emit({
      type: "auto_retry_start",
      attempt: 1,
      maxAttempts: 3,
      delayMs: 0,
      errorMessage: "retry",
    });
    emit({ type: "agent_start" });
    emit({ type: "turn_start" });
  }
  if (message.startsWith("?notify")) {
    for (const method of ["notify", "setStatus", "setWidget", "setTitle", "set_editor_text"]) {
      emit({
        type: "extension_ui_request",
        id: `display-${method}`,
        method,
        message: "Display only",
      });
    }
  }
  if (state.thinkingLevel !== "off") {
    emit({
      type: "message_update",
      usage: { ...usage(), output: 0, totalTokens: 120 },
      assistantMessageEvent: { type: "thinking_delta", contentIndex: 0, delta: "thinking..." },
    });
  }
  emit({
    type: "message_update",
    usage: { ...usage(), output: 0, totalTokens: 120 },
    assistantMessageEvent: { type: "text_delta", contentIndex: 1, delta: reply },
  });
  settle(finishMessage(reply, message.startsWith("!error") ? "error" : "stop"));
  if (message.startsWith("!branch")) {
    state.entries.push({
      type: "message",
      id: "off-branch",
      parentId: state.entries[0]?.id ?? null,
      message: { role: "assistant", content: [{ type: "text", text: "Wrong branch" }] },
    });
  }
};

const holdTurn = () => {
  emit({ type: "agent_start" });
  emit({ type: "turn_start" });
  emit({
    type: "message_update",
    usage: { ...usage(), output: 0, totalTokens: 120 },
    assistantMessageEvent: { type: "text_delta", contentIndex: 0, delta: "Working..." },
  });
};

const finishHeld = (reply) => {
  emit({
    type: "message_update",
    usage: { ...usage(), output: 0, totalTokens: 120 },
    assistantMessageEvent: { type: "text_delta", contentIndex: 1, delta: reply },
  });
  settle(finishMessage(reply));
};

process.stdin.setEncoding("utf8");
let buffered = "";
process.stdin.on("data", (chunk) => {
  buffered += chunk;
  for (;;) {
    const index = buffered.indexOf("\n");
    if (index < 0) break;
    const line = buffered.slice(0, index).replace(/\r$/, "");
    buffered = buffered.slice(index + 1);
    if (!line.trim()) continue;
    let command;
    try {
      command = JSON.parse(line);
    } catch (error) {
      respond(undefined, "parse", false, undefined, `Failed to parse command: ${String(error)}`);
      continue;
    }
    const { id, type } = command;
    switch (type) {
      case "switch_session": {
        if (command.sessionPath === "/cancelled.jsonl") {
          respond(id, type, true, { cancelled: true });
          break;
        }
        state.sessionFile = command.sessionPath;
        NodeFS.mkdirSync(NodePath.dirname(state.sessionFile), { recursive: true });
        NodeFS.closeSync(NodeFS.openSync(state.sessionFile, "a"));
        const saved = NodeFS.readFileSync(state.sessionFile, "utf8").trim();
        state.entries = saved ? saved.split("\n").map((line) => JSON.parse(line)) : [];
        state.leafId = state.entries.at(-1)?.id ?? null;
        respond(id, type, true, { cancelled: false });
        break;
      }
      case "get_state":
        respond(id, type, true, {
          model: state.model,
          thinkingLevel: state.thinkingLevel,
          isStreaming: state.held !== null,
          isCompacting: false,
          sessionFile: state.sessionFile,
          sessionId: state.sessionId,
          ...(state.sessionName === undefined ? {} : { sessionName: state.sessionName }),
          messageCount: state.entries.length,
          pendingMessageCount: 0,
        });
        break;
      case "get_available_models":
        respond(id, type, true, { models });
        break;
      case "get_available_thinking_levels": {
        const levels =
          state.model.provider === "custom"
            ? ["off", "low"]
            : state.model.provider === "openai"
              ? ["off", "medium"]
              : ["off", "low", "medium", "high"];
        respond(id, type, true, { levels });
        break;
      }
      case "set_model": {
        const model = models.find(
          (entry) => entry.provider === command.provider && entry.id === command.modelId,
        );
        if (!model) {
          respond(id, type, false, undefined, `Model not found: ${command.modelId}`);
          break;
        }
        state.model = model;
        pushMetadata("model_change");
        respond(id, type, true, model);
        break;
      }
      case "set_thinking_level":
        state.thinkingLevel = command.level;
        pushMetadata("thinking_level_change");
        respond(id, type, true);
        break;
      case "get_session_stats":
        respond(id, type, true, {
          sessionFile: state.sessionFile,
          sessionId: state.sessionId,
          totalMessages: state.entries.length,
          tokens: {
            input: 50000,
            output: 10000,
            cacheRead: 40000,
            cacheWrite: 5000,
            total: 105000,
          },
          cost: 0.45,
          contextUsage: { tokens: 60000, contextWindow: state.model.contextWindow, percent: 30 },
        });
        break;
      case "get_entries":
        respond(id, type, true, {
          entries: state.entries,
          leafId: state.leafId,
        });
        break;
      case "prompt": {
        const message = String(command.message ?? "");
        if (message.startsWith("!exit")) process.exit(0);
        if (message.startsWith("!reject")) {
          respond(id, type, false, undefined, "Preflight rejected");
          break;
        }
        if (message.startsWith("!handled")) {
          respond(id, type, true);
          break;
        }
        const entryId = pushEntry({
          role: "user",
          content: command.images?.length
            ? [{ type: "text", text: message }, ...command.images]
            : message,
        });
        state.held = { message };
        // Like Pi, acceptance precedes the asynchronously streamed run.
        respond(id, type, true);
        setImmediate(() => {
          const userMessage = state.entries.find((entry) => entry.id === entryId).message;
          emit({ type: "message_start", message: userMessage });
          emit({ type: "message_end", message: userMessage });
          if (message.startsWith("!hold")) {
            holdTurn();
          } else if (/^\?(select|confirm|input|editor):/.test(message)) {
            const method = message.slice(1, message.indexOf(":"));
            state.held.ui = { id: `ui-${entryId}`, method };
            emit({ type: "agent_start" });
            emit({ type: "turn_start" });
            emit({
              type: "extension_ui_request",
              id: state.held.ui.id,
              method,
              title: "Pick one",
              message: "Continue?",
              options: ["alpha", "beta"],
            });
          } else {
            runTurn(
              message,
              message.startsWith("!tool") ? "tool output handled" : `echo: ${message}`,
            );
          }
        });
        break;
      }
      case "steer": {
        pushEntry({
          role: "user",
          content: command.images?.length
            ? [{ type: "text", text: command.message }, ...command.images]
            : command.message,
        });
        if (state.held) finishHeld(`steered: ${command.message}`);
        respond(id, type, true);
        break;
      }
      case "follow_up":
        respond(id, type, true);
        break;
      case "abort": {
        if (state.held) {
          settle(finishMessage("", "aborted"));
        }
        respond(id, type, true);
        break;
      }
      case "extension_ui_response": {
        const held = state.held;
        if (held?.ui && held.ui.id === command.id) {
          const value =
            command.cancelled === true
              ? "cancelled"
              : held.ui.method === "confirm"
                ? String(command.confirmed === true)
                : String(command.value);
          finishHeld(`chose: ${value}`);
        }
        break;
      }
      default:
        respond(id, type, false, undefined, `Unknown command: ${type}`);
    }
  }
});
process.stdin.on("end", () => process.exit(0));
