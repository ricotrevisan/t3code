#!/usr/bin/env node
// @effect-diagnostics nodeBuiltinImport:off globalTimers:off
import * as NodeFS from "node:fs";
import * as NodePath from "node:path";

if (process.argv.includes("--version")) {
  process.stdout.write("prime-rpc-mock 0.0.1\n");
  process.exit(0);
}

const LINE_SEPARATOR = "\u2028";
const exitLogPath = process.env.T3_PRIME_MOCK_EXIT_LOG_PATH;
const requestLogPath = process.env.T3_PRIME_MOCK_REQUEST_LOG_PATH;
const approvalSideEffectPath = process.env.T3_PRIME_MOCK_APPROVAL_SIDE_EFFECT_PATH;
const approvalHandshake = process.env.T3_PRIME_MOCK_APPROVAL_HANDSHAKE ?? "valid";
const suspendAfterAbort = process.env.T3_PRIME_MOCK_SUSPEND_AFTER_ABORT === "1";
const argv = process.argv.slice(2);
const openRouterCatalogExtensionLoaded = argv.some(
  (value, index) =>
    argv[index - 1] === "--extension" && value.endsWith("/t3-openrouter-catalog.ts"),
);

function valueAfter(flag: string): string | undefined {
  const index = argv.indexOf(flag);
  return index >= 0 ? argv[index + 1] : undefined;
}

function valuesAfter(flag: string): Array<string> {
  return argv.flatMap((value, index) =>
    value === flag && argv[index + 1] ? [argv[index + 1]!] : [],
  );
}

const noSession = argv.includes("--no-session");
const sessionDir = valueAfter("--session-dir") ?? process.cwd();
const resumePath = valueAfter("--resume");
const sessionFile = resumePath
  ? NodePath.resolve(resumePath)
  : NodePath.join(sessionDir, "prime-mock-session.jsonl");
if (!noSession) {
  NodeFS.mkdirSync(sessionDir, { recursive: true });
  NodeFS.writeFileSync(sessionFile, "", { flag: "a" });
}

let streaming = false;
let sessionInputSuspended = false;
let turnGeneration = 0;
let delayedTimer: ReturnType<typeof setTimeout> | undefined;
let pendingApprovalId: string | undefined;
let pendingUserInputId: string | undefined;
const steerQueue: string[] = [];
let heartbeats: Array<{
  job: {
    id: string;
    status: string;
    source?: string;
    label?: string;
    prompt?: string;
  };
}> = [];
let currentModel: { id: string; name: string; provider: string } = {
  id: "gpt-5.6-sol",
  name: "GPT-5.6 Sol",
  provider: "openai-codex",
};
let thinkingLevel = "medium";
const sessionId = process.env.T3_PRIME_MOCK_SESSION_ID?.trim() || "prime-mock-session";

const MOCK_MODELS = [
  {
    id: "gpt-5.6-sol",
    name: "GPT-5.6 Sol",
    provider: "openai-codex",
    thinkingLevelMap: { xhigh: "xhigh", minimal: null, max: "max" },
  },
  {
    id: "claude-sonnet-4",
    name: "Claude Sonnet 4",
    provider: "anthropic",
    thinkingLevelMap: { low: "low", medium: "medium", high: "high" },
  },
  {
    id: "no-think",
    name: "No Think",
    provider: "local",
  },
] as const;

function logExit(reason: string): void {
  if (!exitLogPath) {
    return;
  }
  NodeFS.appendFileSync(exitLogPath, `${reason}\n`, "utf8");
}

function logRequest(command: Record<string, unknown>): void {
  if (!requestLogPath) {
    return;
  }
  NodeFS.appendFileSync(requestLogPath, `${JSON.stringify({ args: argv, command })}\n`, "utf8");
}

function loadLastMessage(): string | undefined {
  if (!NodeFS.existsSync(sessionFile)) {
    return undefined;
  }
  const lines = NodeFS.readFileSync(sessionFile, "utf8")
    .split("\n")
    .filter((line) => line.trim().length > 0);
  for (let index = lines.length - 1; index >= 0; index -= 1) {
    try {
      const parsed = JSON.parse(lines[index]!) as { message?: unknown };
      if (typeof parsed.message === "string" && parsed.message.length > 0) {
        return parsed.message;
      }
    } catch {
      // Ignore malformed session lines in the mock fixture.
    }
  }
  return undefined;
}

function persistMessage(message: string): void {
  if (noSession || message.length === 0) {
    return;
  }
  NodeFS.mkdirSync(NodePath.dirname(sessionFile), { recursive: true });
  NodeFS.appendFileSync(sessionFile, `${JSON.stringify({ message })}\n`, "utf8");
}

function send(value: unknown): void {
  process.stdout.write(`${JSON.stringify(value)}\n`);
}

function respond(
  id: string | undefined,
  command: string,
  success: boolean,
  extra?: Record<string, unknown>,
): void {
  send({ id, type: "response", command, success, ...extra });
}

function emitTextTurn(text: string): void {
  send({ type: "agent_start" });
  send({
    type: "message_update",
    assistantMessageEvent: {
      type: "text_delta",
      contentIndex: 0,
      delta: text,
    },
  });
  send({
    type: "agent_end",
    messages: [{ role: "assistant", content: [{ type: "text", text }] }],
  });
}

function emitToolTurn(text: string): void {
  send({ type: "agent_start" });
  send({
    type: "message_update",
    assistantMessageEvent: {
      type: "thinking_delta",
      contentIndex: 0,
      delta: "Prime is thinking",
    },
  });
  send({
    type: "tool_execution_start",
    toolCallId: "prime-tool-1",
    toolName: "ipython",
    args: { code: "1 + 1" },
  });
  send({
    type: "tool_execution_update",
    toolCallId: "prime-tool-1",
    toolName: "ipython",
    args: { code: "1 + 1" },
    partialResult: { content: [{ type: "text", text: "2" }] },
  });
  send({
    type: "tool_execution_update",
    toolCallId: "prime-tool-1",
    toolName: "ipython",
    args: { code: "1 + 1" },
    partialResult: { content: [{ type: "text", text: "2\n3" }] },
  });
  send({
    type: "tool_execution_end",
    toolCallId: "prime-tool-1",
    toolName: "ipython",
    result: { content: [{ type: "text", text: "2\n3" }] },
    isError: false,
  });
  send({
    type: "message_update",
    assistantMessageEvent: {
      type: "text_delta",
      contentIndex: 0,
      delta: text,
    },
  });
  send({
    type: "agent_end",
    messages: [
      {
        role: "assistant",
        content: [{ type: "text", text }],
        usage: {
          input: 10,
          output: 4,
          cacheRead: 0,
          cacheWrite: 0,
          totalTokens: 14,
        },
      },
    ],
  });
}

function finishTurn(emit: () => void): void {
  streaming = false;
  delayedTimer = undefined;
  emit();
}

function handle(command: Record<string, unknown>): void {
  logRequest(command);
  const id = typeof command.id === "string" ? command.id : undefined;
  switch (command.type) {
    case "get_state":
      respond(id, "get_state", true, {
        data: {
          model: currentModel,
          thinkingLevel,
          isStreaming: streaming,
          isCompacting: false,
          sessionFile,
          sessionId,
          messageCount: 0,
          unfinishedActionCount: 0,
          sessionActions: { queuedCount: 0, steering: [], followUps: [] },
        },
      });
      return;
    case "get_available_models":
      respond(id, "get_available_models", true, {
        data: {
          models: [
            ...MOCK_MODELS,
            ...(openRouterCatalogExtensionLoaded
              ? [
                  {
                    id: "stealth/ox-alpha",
                    name: "Ox Alpha",
                    provider: "openrouter",
                    thinkingLevelMap: {
                      off: null,
                      minimal: null,
                      low: "low",
                      medium: null,
                      high: "high",
                      xhigh: null,
                      max: "max",
                    },
                  },
                ]
              : []),
          ],
        },
      });
      return;
    case "list_heartbeats":
      respond(id, "list_heartbeats", true, {
        data: { heartbeats },
      });
      return;
    case "get_commands": {
      const extensionPath =
        valuesAfter("--extension").find((value) => value.endsWith("/t3-approval-v1.ts")) ??
        valueAfter("--extension");
      if (approvalHandshake === "malformed") {
        respond(id, "get_commands", true, { data: { commands: "invalid" } });
        return;
      }
      respond(id, "get_commands", true, {
        data: {
          commands:
            extensionPath && approvalHandshake !== "missing"
              ? [
                  {
                    name: "t3-approval-v1",
                    description: "T3 approval bridge protocol 1",
                    source: "extension",
                    sourceInfo: {
                      path:
                        approvalHandshake === "wrong-path"
                          ? NodePath.resolve(extensionPath, "..", "rogue.ts")
                          : NodePath.resolve(extensionPath),
                      source: approvalHandshake === "wrong-source" ? "settings" : "cli",
                      scope: "temporary",
                      origin: "top-level",
                    },
                  },
                ]
              : [],
        },
      });
      return;
    }
    case "set_model": {
      const provider = typeof command.provider === "string" ? command.provider.trim() : "";
      const modelId = typeof command.modelId === "string" ? command.modelId.trim() : "";
      if (!provider || !modelId) {
        respond(id, "set_model", false, { error: "provider and modelId are required" });
        return;
      }
      currentModel = {
        id: modelId,
        name: modelId,
        provider,
      };
      respond(id, "set_model", true, { data: currentModel });
      return;
    }
    case "set_thinking_level": {
      const level = typeof command.level === "string" ? command.level.trim() : "";
      if (!level) {
        respond(id, "set_thinking_level", false, { error: "level is required" });
        return;
      }
      thinkingLevel = level;
      respond(id, "set_thinking_level", true);
      return;
    }
    case "steer": {
      if (!streaming) {
        respond(id, "steer", false, { error: "Agent is not streaming." });
        return;
      }
      const message = typeof command.message === "string" ? command.message : "";
      steerQueue.push(message);
      respond(id, "steer", true);
      return;
    }
    case "prompt": {
      if (sessionInputSuspended && command.streamingBehavior === undefined) {
        respond(id, "prompt", false, {
          error: "Cannot admit a session action while queued session input is suspended.",
        });
        return;
      }
      sessionInputSuspended = false;
      if (streaming) {
        respond(id, "prompt", false, {
          error: "Agent is already streaming. Specify streamingBehavior to queue the message.",
        });
        return;
      }
      streaming = true;
      const generation = ++turnGeneration;
      respond(id, "prompt", true);
      const message = typeof command.message === "string" ? command.message : "";
      const stillCurrent = () => generation === turnGeneration && streaming;
      if (message === "text input") {
        pendingUserInputId = "prime-input-1";
        send({ type: "agent_start" });
        send({
          type: "extension_ui_request",
          id: pendingUserInputId,
          method: "input",
          title: "Name release",
          placeholder: "v1.2.3",
        });
        return;
      }
      if (message === "editor input") {
        pendingUserInputId = "prime-editor-1";
        send({ type: "agent_start" });
        send({
          type: "extension_ui_request",
          id: pendingUserInputId,
          method: "editor",
          title: "Edit release notes",
          prefill: "Old notes",
        });
        return;
      }
      if (message === "select input") {
        pendingUserInputId = "prime-select-1";
        send({ type: "agent_start" });
        send({
          type: "extension_ui_request",
          id: pendingUserInputId,
          method: "select",
          title: "Choose environment",
          options: ["dev", "prod"],
        });
        return;
      }
      if (message === "approval then end") {
        send({ type: "agent_start" });
        send({
          type: "extension_ui_request",
          id: "prime-approval-end",
          method: "confirm",
          title: "Allow Prime Agent tool?",
          message: 'ipython\n\n{"code":"finish"}',
        });
        streaming = false;
        send({
          type: "agent_end",
          messages: [{ role: "assistant", content: [], stopReason: "cancelled" }],
        });
        return;
      }
      if (message === "approval then exit") {
        pendingApprovalId = "prime-approval-exit";
        send({ type: "agent_start" });
        process.stdout.write(
          `${JSON.stringify({
            type: "extension_ui_request",
            id: pendingApprovalId,
            method: "confirm",
            title: "Allow Prime Agent tool?",
            message: 'ipython\n\n{"code":"exit"}',
          })}\n`,
          () => process.exit(7),
        );
        return;
      }
      if (message === "approval side effect") {
        pendingApprovalId = "prime-approval-1";
        send({ type: "agent_start" });
        send({
          type: "tool_execution_start",
          toolCallId: "prime-approval-tool-1",
          toolName: "ipython",
          args: { code: "write approval sentinel" },
        });
        send({
          type: "extension_ui_request",
          id: pendingApprovalId,
          method: "confirm",
          title: "Allow Prime Agent tool?",
          message: 'ipython\n\n{"code":"write approval sentinel"}',
        });
        return;
      }
      if (message === "arm heartbeat") {
        heartbeats = [
          {
            job: {
              id: "hb-1",
              status: "active",
              source: "rlm_heartbeat",
              label: "ci-watch",
              prompt: "Check CI",
            },
          },
        ];
        send({ type: "agent_start" });
        finishTurn(() => {
          send({
            type: "message_update",
            assistantMessageEvent: {
              type: "text_delta",
              contentIndex: 0,
              delta: "Heartbeat armed",
            },
          });
          send({
            type: "agent_end",
            messages: [
              {
                role: "assistant",
                content: [{ type: "text", text: "Heartbeat armed" }],
              },
            ],
          });
          delayedTimer = setTimeout(() => {
            send({ type: "agent_start" });
            send({
              type: "message_update",
              assistantMessageEvent: {
                type: "text_delta",
                contentIndex: 0,
                delta: "Heartbeat tick",
              },
            });
            send({
              type: "agent_end",
              messages: [
                {
                  role: "assistant",
                  content: [{ type: "text", text: "Heartbeat tick" }],
                },
              ],
            });
          }, 80);
        });
        return;
      }
      if (message === "spawn subagents") {
        send({ type: "agent_start" });
        send({
          type: "rlm_child_update",
          child: {
            id: "prime-sub-1",
            sessionName: "audit-renderer",
            model: "openrouter/ox-alpha",
            label: "Audit the pie renderer code",
            status: "queued",
            sessionDir: "/tmp/prime-sub-1",
          },
        });
        send({
          type: "rlm_child_update",
          child: {
            id: "prime-sub-2",
            sessionName: "audit-tests-scripts",
            model: "openrouter/ox-alpha",
            label: "Audit the new test files",
            status: "queued",
            sessionDir: "/tmp/prime-sub-2",
          },
        });
        send({
          type: "rlm_child_update",
          child: {
            id: "prime-sub-1",
            sessionName: "audit-renderer",
            model: "openrouter/ox-alpha",
            label: "Audit the pie renderer code",
            status: "running",
            tokenCount: 4200,
            toolUseCount: 1,
            sessionDir: "/tmp/prime-sub-1",
            activity: { kind: "executing", toolName: "ipython" },
          },
        });
        // A text-delta snapshot with identical roster state must be deduped.
        send({
          type: "rlm_child_update",
          child: {
            id: "prime-sub-1",
            sessionName: "audit-renderer",
            model: "openrouter/ox-alpha",
            label: "Audit the pie renderer code",
            status: "running",
            answerPreview: "Reading pie-definition.ts",
            tokenCount: 4200,
            toolUseCount: 1,
            sessionDir: "/tmp/prime-sub-1",
            activity: { kind: "executing", toolName: "ipython" },
          },
        });
        send({
          type: "rlm_child_update",
          child: {
            id: "prime-sub-1",
            sessionName: "audit-renderer",
            model: "openrouter/ox-alpha",
            label: "Audit the pie renderer code",
            status: "done",
            answerPreview: "Found one dead helper in normalize.js",
            tokenCount: 9100,
            toolUseCount: 3,
            durationMs: 2500,
            sessionDir: "/tmp/prime-sub-1",
          },
        });
        send({
          type: "message_update",
          assistantMessageEvent: {
            type: "text_delta",
            contentIndex: 0,
            delta: "spawned subagents",
          },
        });
        setImmediate(() => {
          if (!stillCurrent()) {
            return;
          }
          // Parent agent_end while a child is still running. The inbound
          // child report then starts a second parent text cycle.
          finishTurn(() => {
            send({ type: "agent_end", messages: [{ role: "assistant", content: [] }] });
          });
          setImmediate(() => {
            send({
              type: "message_end",
              message: {
                role: "custom",
                customType: "agent_message",
                content:
                  "[from child:audit-renderer]\nAgent-to-agent message received.\n\nFound one dead helper in normalize.js",
                display: true,
                details: {
                  id: "agentmsg_child1",
                  message: "Found one dead helper in normalize.js",
                  from: { sessionName: "audit-renderer", sessionId: "prime-sub-1" },
                  fromRelationship: "child",
                },
              },
            });
            send({
              type: "rlm_child_update",
              child: {
                id: "prime-sub-2",
                sessionName: "audit-tests-scripts",
                model: "openrouter/ox-alpha",
                label: "Audit the new test files",
                status: "error",
                durationMs: 3000,
                sessionDir: "/tmp/prime-sub-2",
                error: "Test suite could not start",
              },
            });
            streaming = true;
            send({ type: "agent_start" });
            send({
              type: "message_update",
              assistantMessageEvent: {
                type: "text_delta",
                contentIndex: 0,
                delta: "## Verdict\nBoth audits finished",
              },
            });
            finishTurn(() => {
              send({
                type: "agent_end",
                messages: [
                  {
                    role: "assistant",
                    content: [{ type: "text", text: "## Verdict\nBoth audits finished" }],
                  },
                ],
              });
            });
          });
        });
        return;
      }
      if (message === "late child report") {
        send({ type: "agent_start" });
        send({
          type: "rlm_child_update",
          child: {
            id: "prime-sub-late",
            sessionName: "late-reporter",
            model: "openrouter/ox-alpha",
            label: "Report after settling",
            status: "running",
            sessionDir: "/tmp/prime-sub-late",
            activity: { kind: "executing", toolName: "ipython" },
          },
        });
        send({
          type: "message_update",
          assistantMessageEvent: {
            type: "text_delta",
            contentIndex: 0,
            delta: "delegated",
          },
        });
        setImmediate(() => {
          if (!stillCurrent()) {
            return;
          }
          // A terminal roster update can land before the current parent
          // cycle ends and before Prime admits the hidden parent wake. T3 must
          // preserve that reservation across agent_end, including failures.
          send({
            type: "rlm_child_update",
            child: {
              id: "prime-sub-late",
              sessionName: "late-reporter",
              model: "openrouter/ox-alpha",
              label: "Report after settling",
              status: "error",
              repliedSinceTask: false,
              error: "Late child failed with findings",
              tokenCount: 5200,
              toolUseCount: 2,
              durationMs: 1800,
              sessionDir: "/tmp/prime-sub-late",
            },
          });
          finishTurn(() => {
            send({
              type: "agent_end",
              messages: [{ role: "assistant", content: [{ type: "text", text: "delegated" }] }],
            });
          });
          setImmediate(() => {
            send({
              type: "message_end",
              message: {
                role: "custom",
                customType: "agent_message",
                content:
                  "[from child:late-reporter]\nAgent-to-agent message received.\n\nLate findings",
                display: true,
                details: {
                  id: "agentmsg_late1",
                  message: "Late findings",
                  from: { sessionName: "late-reporter", sessionId: "prime-sub-late" },
                  fromRelationship: "child",
                },
              },
            });
            streaming = true;
            send({ type: "agent_start" });
            send({
              type: "message_update",
              assistantMessageEvent: {
                type: "text_delta",
                contentIndex: 0,
                delta: "## Late verdict",
              },
            });
            finishTurn(() => {
              send({
                type: "agent_end",
                messages: [
                  {
                    role: "assistant",
                    content: [{ type: "text", text: "## Late verdict" }],
                  },
                ],
              });
            });
          });
        });
        return;
      }
      if (message === "hold split unicode") {
        const prefix =
          '{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"';
        process.stdout.write(
          Buffer.concat([Buffer.from(prefix, "utf8"), Buffer.from([0xe2, 0x82])]),
        );
        return;
      }
      if (message === "slow turn") {
        send({ type: "agent_start" });
        delayedTimer = setTimeout(() => {
          if (!stillCurrent()) {
            return;
          }
          const slowText = ["too late", ...steerQueue].join(" ");
          finishTurn(() => {
            send({
              type: "message_update",
              assistantMessageEvent: {
                type: "text_delta",
                contentIndex: 0,
                delta: slowText,
              },
            });
            send({
              type: "agent_end",
              messages: [{ role: "assistant", content: [{ type: "text", text: slowText }] }],
            });
          });
        }, 2_000);
        return;
      }
      if (message === "fail prompt") {
        setImmediate(() => {
          if (!stillCurrent()) {
            return;
          }
          finishTurn(() => {
            send({ type: "agent_start" });
            send({
              type: "agent_end",
              messages: [
                {
                  role: "assistant",
                  content: [],
                  stopReason: "error",
                  errorMessage: "Prime model rejected the request.",
                },
              ],
            });
          });
        });
        return;
      }
      const recalled = message === "recall last" ? loadLastMessage() : undefined;
      if (message !== "recall last") {
        persistMessage(message);
      }
      const text =
        recalled !== undefined
          ? recalled
          : message === "recall last"
            ? "nothing"
            : message === "emit line separator"
              ? `hello${LINE_SEPARATOR}world`
              : message === "emit tools"
                ? "hello from tools"
                : "hello from mock";
      setImmediate(() => {
        if (!stillCurrent()) {
          return;
        }
        finishTurn(() => {
          if (message === "emit tools") {
            emitToolTurn(text);
            return;
          }
          emitTextTurn(text);
        });
      });
      return;
    }
    case "extension_ui_response": {
      if (typeof command.id === "string" && command.id === pendingUserInputId) {
        const value = typeof command.value === "string" ? command.value : "cancelled";
        pendingUserInputId = undefined;
        streaming = false;
        send({
          type: "message_update",
          assistantMessageEvent: {
            type: "text_delta",
            contentIndex: 0,
            delta: value,
          },
        });
        send({
          type: "agent_end",
          messages: [{ role: "assistant", content: [{ type: "text", text: value }] }],
        });
        return;
      }
      if (typeof command.id !== "string" || command.id !== pendingApprovalId) {
        return;
      }
      const approved = command.confirmed === true;
      pendingApprovalId = undefined;
      streaming = false;
      if (approved && approvalSideEffectPath) {
        NodeFS.mkdirSync(NodePath.dirname(approvalSideEffectPath), { recursive: true });
        NodeFS.writeFileSync(approvalSideEffectPath, "approved\n", "utf8");
      }
      send({
        type: "tool_execution_end",
        toolCallId: "prime-approval-tool-1",
        toolName: "ipython",
        result: approved ? { content: [{ type: "text", text: "approved" }] } : undefined,
        isError: !approved,
      });
      send({
        type: "agent_end",
        messages: [
          {
            role: "assistant",
            content: approved ? [{ type: "text", text: "approved" }] : [],
            ...(approved ? {} : { stopReason: "cancelled" }),
          },
        ],
      });
      return;
    }
    case "abort": {
      if (delayedTimer !== undefined) {
        clearTimeout(delayedTimer);
        delayedTimer = undefined;
      }
      turnGeneration += 1;
      const wasStreaming = streaming;
      streaming = false;
      sessionInputSuspended = suspendAfterAbort;
      respond(id, "abort", true);
      if (wasStreaming) {
        send({
          type: "agent_end",
          messages: [
            {
              role: "assistant",
              content: [],
              stopReason: "aborted",
            },
          ],
        });
      }
      return;
    }
    default:
      respond(id, String(command.type ?? "unknown"), false, {
        error: "Unsupported mock command",
      });
  }
}

let buffer = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk: string) => {
  buffer += chunk;
  while (true) {
    const newline = buffer.indexOf("\n");
    if (newline < 0) {
      break;
    }
    const line = buffer.slice(0, newline).replace(/\r$/, "");
    buffer = buffer.slice(newline + 1);
    if (line.length === 0) {
      continue;
    }
    handle(JSON.parse(line) as Record<string, unknown>);
  }
});

process.stdin.on("end", () => {
  logExit("stdin-eof");
  process.exit(0);
});

process.once("SIGTERM", () => {
  logExit("SIGTERM");
  process.exit(0);
});

process.once("SIGINT", () => {
  logExit("SIGINT");
  process.exit(0);
});
