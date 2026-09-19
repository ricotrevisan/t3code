/**
 * Deterministic ACP v1 agent used by the real-process bridge conformance test.
 * It speaks newline-delimited JSON-RPC on stdio and records decoded wire input
 * to the path in ACP_CONFORMANCE_AUDIT.
 */
import * as NodeFS from "node:fs";

const auditPath = process.env.ACP_CONFORMANCE_AUDIT;
const audit = (entry) => {
  if (auditPath) NodeFS.appendFileSync(auditPath, `${JSON.stringify(entry)}\n`);
};

const sessions = new Map();
const pendingPrompts = new Map();
const pendingPermissions = new Map();
let nextSession = 0;
let nextPermission = 0;

const models = [
  { value: "vendor/model@opaque:alpha", name: "Opaque Alpha" },
  { value: "urn:acp:model:%2Fbeta?x=1", name: "Opaque Beta" },
];
const reasoningFor = (model) =>
  model === models[1].value ? ["brief@beta", "deep@opaque/value"] : ["brief@alpha", "deep@alpha"];
const configOptions = (model, reasoning = reasoningFor(model)[0]) => [
  {
    type: "select",
    id: "model",
    name: "Model",
    category: "model",
    currentValue: model,
    options: models,
  },
  {
    type: "select",
    id: "reasoning_effort",
    name: "Reasoning",
    category: "thought_level",
    currentValue: reasoning,
    options: reasoningFor(model).map((value) => ({ value, name: value })),
  },
];

const send = (message) =>
  process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", ...message })}\n`);
const respond = (id, result) => send({ id, result });
const notify = (sessionId, update) =>
  send({ method: "session/update", params: { sessionId, update } });
const fail = (id, code, message) => send({ id, error: { code, message } });
const promptText = (params) =>
  (params?.prompt ?? [])
    .filter((content) => content?.type === "text")
    .map((content) => content.text)
    .join("");

const handleRequest = (message) => {
  audit({ kind: message.method ? "request" : "response", message });
  const { id, method, params } = message;

  if (!method) {
    const permission = pendingPermissions.get(String(id));
    if (permission) {
      pendingPermissions.delete(String(id));
      audit({ kind: "permission-resolution", result: message.result });
      pendingPrompts.delete(String(permission.promptId));
      respond(permission.promptId, { stopReason: "end_turn" });
    }
    return;
  }

  switch (method) {
    case "initialize":
      respond(id, {
        protocolVersion: 1,
        agentInfo: { name: "acp-conformance-harness", version: "1.0.0" },
        agentCapabilities: {
          loadSession: false,
          sessionCapabilities: { resume: {}, close: {} },
        },
      });
      return;

    case "session/new": {
      const sessionId = `new-session-${++nextSession}`;
      const model = models[0].value;
      sessions.set(sessionId, { model, reasoning: reasoningFor(model)[0] });
      respond(id, { sessionId, configOptions: configOptions(model) });
      return;
    }

    case "session/resume": {
      const model = models[0].value;
      sessions.set(params.sessionId, { model, reasoning: reasoningFor(model)[0], resumed: true });
      respond(id, { configOptions: configOptions(model) });
      return;
    }

    case "session/set_config_option": {
      const state = sessions.get(params.sessionId);
      if (!state) return fail(id, -32001, "unknown session");
      if (params.configId === "model") {
        state.model = params.value;
        state.reasoning = reasoningFor(state.model)[0];
      } else if (params.configId === "reasoning_effort") {
        state.reasoning = params.value;
      }
      respond(id, { configOptions: configOptions(state.model, state.reasoning) });
      return;
    }

    case "session/prompt": {
      const text = promptText(params);
      pendingPrompts.set(String(id), { id, sessionId: params.sessionId, text });
      if (text === "canonical updates") {
        notify(params.sessionId, {
          sessionUpdate: "agent_thought_chunk",
          messageId: "thought-main",
          content: { type: "text", text: "considering" },
        });
        notify(params.sessionId, {
          sessionUpdate: "tool_call",
          toolCallId: "generic-tool-1",
          title: "Generic lifecycle",
          kind: "other",
          status: "pending",
          rawInput: { opaque: true },
        });
        notify(params.sessionId, {
          sessionUpdate: "tool_call_update",
          toolCallId: "generic-tool-1",
          status: "completed",
          rawOutput: { answer: 42 },
        });
        notify(params.sessionId, { sessionUpdate: "usage_update", used: 37, size: 128 });
        notify(params.sessionId, {
          sessionUpdate: "agent_message_chunk",
          messageId: "assistant-main",
          content: { type: "text", text: "answer" },
        });
        notify(params.sessionId, {
          sessionUpdate: "agent_message_chunk",
          messageId: "assistant-main",
          content: {
            type: "image",
            data: "aW1hZ2U=",
            mimeType: "image/png",
            uri: "data:fixture",
          },
        });
        pendingPrompts.delete(String(id));
        respond(id, { stopReason: "end_turn" });
        return;
      }
      if (text === "permission turn") {
        const permissionId = `permission-${++nextPermission}`;
        pendingPermissions.set(permissionId, { promptId: id, sessionId: params.sessionId });
        send({
          id: permissionId,
          method: "session/request_permission",
          params: {
            sessionId: params.sessionId,
            options: [
              { optionId: "allow-once-wire", name: "Allow once", kind: "allow_once" },
              { optionId: "reject-once-wire", name: "Reject", kind: "reject_once" },
            ],
            toolCall: {
              toolCallId: "permission-tool",
              title: "Run conformance command",
              kind: "execute",
              status: "pending",
            },
          },
        });
        return;
      }
      if (text === "hold for cancel") {
        notify(params.sessionId, {
          sessionUpdate: "agent_message_chunk",
          messageId: "assistant-held",
          content: { type: "text", text: "holding" },
        });
        return;
      }
      notify(params.sessionId, {
        sessionUpdate: "agent_message_chunk",
        messageId: `assistant-${id}`,
        content: { type: "text", text: `echo:${text}` },
      });
      pendingPrompts.delete(String(id));
      respond(id, { stopReason: "end_turn" });
      return;
    }

    case "session/cancel":
      for (const [promptId, prompt] of pendingPrompts) {
        if (prompt.sessionId !== params.sessionId) continue;
        pendingPrompts.delete(promptId);
        for (const [permissionId, permission] of pendingPermissions) {
          if (String(permission.promptId) === promptId) pendingPermissions.delete(permissionId);
        }
        respond(prompt.id, { stopReason: "cancelled" });
      }
      return;

    case "session/close":
      sessions.delete(params.sessionId);
      respond(id, {});
      return;

    default:
      fail(id, -32601, `unknown method: ${method}`);
  }
};

process.stdin.setEncoding("utf8");
let buffer = "";
process.stdin.on("data", (chunk) => {
  buffer += chunk;
  for (;;) {
    const newline = buffer.indexOf("\n");
    if (newline < 0) break;
    const line = buffer.slice(0, newline).replace(/\r$/, "");
    buffer = buffer.slice(newline + 1);
    if (!line.trim()) continue;
    try {
      handleRequest(JSON.parse(line));
    } catch (error) {
      audit({ kind: "harness-error", error: String(error?.stack ?? error), line });
      fail(null, -32700, "parse error");
    }
  }
});
process.stdin.on("end", () => {
  audit({ kind: "eof", trailingBytes: buffer.length });
});
