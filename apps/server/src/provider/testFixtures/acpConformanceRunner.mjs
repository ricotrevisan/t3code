/**
 * Real-process conformance for the generic ACP stdio adapter.
 * All completion waits are protocol events, RPC replies, or process exit.
 */
import * as NodeFS from "node:fs";
import * as NodeOS from "node:os";
import * as NodePath from "node:path";
import * as NodeURL from "node:url";

import * as Effect from "effect/Effect";
import * as Fiber from "effect/Fiber";
import * as Stream from "effect/Stream";

const { makeExternalProviderDriver } = await import(
  new URL("../ExternalProviderDriver.ts", import.meta.url).href
);
const { makeExternalProviderProcessSupervisor } = await import(
  new URL("../ExternalProviderProcessSupervisor.ts", import.meta.url).href
);
const { AcpStdioAdapter } = await import("@t3tools/provider-adapter-acp");
const { defineAcpStdioAdapterV1 } = AcpStdioAdapter;

const harnessPath = NodeURL.fileURLToPath(new URL("./acpHarness.mjs", import.meta.url));
const tempDirectory = NodeFS.mkdtempSync(NodePath.join(NodeOS.tmpdir(), "t3-acp-conformance-"));
const rawAuditPath = NodePath.join(tempDirectory, "raw.ndjson");
const adapterAuditPath = NodePath.join(tempDirectory, "adapter.ndjson");
const result = { checks: [], allEvents: [] };
const check = (name, ok, detail) => {
  result.checks.push({ name, ok: Boolean(ok), ...(detail === undefined ? {} : { detail }) });
};
const terminal = (event) => event.type === "turn.completed" || event.type === "turn.aborted";
const readAudit = (path) =>
  NodeFS.existsSync(path)
    ? NodeFS.readFileSync(path, "utf8")
        .split("\n")
        .filter(Boolean)
        .map((line) => JSON.parse(line))
    : [];
const packageIdentity = {
  id: "acp-conformance",
  version: "1.0.0",
  protocolVersion: 1,
};

const manifest = {
  protocolVersion: 1,
  id: packageIdentity.id,
  version: packageIdentity.version,
  driver: "acpConformance",
  displayName: "ACP Conformance",
  hostProtocol: { minimum: 1, maximum: 1 },
  transport: {
    kind: "supervised-stdio",
    protocol: "acp-v1",
    sessionConcurrency: "multiplexed",
  },
  capabilities: [
    "session.resume",
    "turn.interrupt",
    "request.approval",
    "model.discovery",
    "model.switch",
    "reasoning.selection",
    "stream.reasoning",
    "stream.tool-lifecycle",
    "stream.usage",
    "stream.context",
    "input.attachments",
    "conversation.rollback",
    "request.structured-input",
  ],
  configSchema: {
    type: "object",
    properties: {
      command: { type: "string" },
      args: { type: "array", items: { type: "string" } },
      cwd: { type: "string" },
      env: { type: "object", additionalProperties: { type: "string" } },
    },
    required: ["command"],
  },
};
const config = {
  command: process.execPath,
  args: [harnessPath],
  cwd: tempDirectory,
  env: { ACP_CONFORMANCE_AUDIT: adapterAuditPath },
};
const pkg = defineAcpStdioAdapterV1({
  manifest,
  defaultConfig: () => config,
  clientInfo: { name: "t3-acp-conformance", version: "1.0.0" },
});

const program = Effect.gen(function* () {
  // Exercise the same production supervisor directly once. This proves a
  // complete NDJSON frame and natural stdin-EOF exit without adapter mocks.
  const supervisor = yield* makeExternalProviderProcessSupervisor();
  const raw = yield* supervisor.processes.spawn({
    command: process.execPath,
    args: [harnessPath],
    environment: { ACP_CONFORMANCE_AUDIT: rawAuditPath },
    purpose: { kind: "probe" },
  });
  const rawReplyFiber = yield* raw.stdout.pipe(
    Stream.decodeText(),
    Stream.splitLines,
    Stream.filter((line) => line.trim().length > 0),
    Stream.take(1),
    Stream.runCollect,
    Effect.forkChild,
  );
  yield* raw.write(
    new TextEncoder().encode(
      `${JSON.stringify({
        jsonrpc: "2.0",
        id: "raw-initialize",
        method: "initialize",
        params: { protocolVersion: 1, clientCapabilities: {} },
      })}\n`,
    ),
  );
  const rawReplies = Array.from(yield* Fiber.join(rawReplyFiber)).map((line) => JSON.parse(line));
  check(
    "supervisor carries one ACP NDJSON frame",
    rawReplies.length === 1 &&
      rawReplies[0]?.id === "raw-initialize" &&
      rawReplies[0]?.result?.protocolVersion === 1,
  );
  yield* raw.expectExit;
  yield* raw.close;
  check("supervisor observes natural EOF exit", (yield* raw.exitCode) === 0);
  const rawAudit = readAudit(rawAuditPath);
  check(
    "harness receives exact line framing before EOF",
    rawAudit.some((entry) => entry.kind === "request" && entry.message.method === "initialize") &&
      rawAudit.some((entry) => entry.kind === "eof" && entry.trailingBytes === 0),
  );

  const driver = makeExternalProviderDriver(pkg, config);
  const instance = yield* driver.create({
    instanceId: "acp_conformance",
    displayName: "ACP Conformance Instance",
    accentColor: undefined,
    environment: [],
    enabled: true,
    config,
  });
  const threadA = "acp-thread-a";
  const threadB = "acp-thread-b";
  const threadResumed = "acp-thread-resumed";
  const secondModel = "urn:acp:model:%2Fbeta?x=1";
  const secondReasoning = "deep@opaque/value";

  const collectFor = (threadId, predicate = terminal) =>
    instance.adapter.streamEvents.pipe(
      Stream.filter((event) => event.threadId === threadId),
      Stream.takeUntil(predicate),
      Stream.runCollect,
      Effect.forkChild,
    );
  const drain = (fiber) =>
    Fiber.join(fiber).pipe(
      Effect.map((chunk) => {
        const events = Array.from(chunk);
        result.allEvents.push(...events);
        return events;
      }),
    );
  const complete = (threadId, input, extra) =>
    Effect.gen(function* () {
      const fiber = yield* collectFor(threadId);
      yield* Effect.yieldNow;
      const turn = yield* instance.adapter.sendTurn({
        threadId,
        input,
        attachments: [],
        ...extra,
      });
      const events = yield* drain(fiber);
      check(
        `exactly one terminal: ${input}`,
        events.filter(terminal).length === 1 && events.at(-1)?.turnId === turn.turnId,
      );
      return { turn, events };
    });

  const sessionA = yield* instance.adapter.startSession({
    threadId: threadA,
    providerInstanceId: "acp_conformance",
    cwd: tempDirectory,
    runtimeMode: "approval-required",
    modelSelection: {
      instanceId: "acp_conformance",
      model: secondModel,
      options: [{ id: "reasoningEffort", value: secondReasoning }],
    },
  });
  const sessionB = yield* instance.adapter.startSession({
    threadId: threadB,
    providerInstanceId: "acp_conformance",
    cwd: tempDirectory,
    runtimeMode: "full-access",
  });
  const resumeCursor = {
    kind: "acp-v1",
    protocolVersion: 1,
    sessionId: "saved/session:opaque?x=1",
  };
  const resumed = yield* instance.adapter.startSession({
    threadId: threadResumed,
    providerInstanceId: "acp_conformance",
    cwd: tempDirectory,
    runtimeMode: "full-access",
    resumeCursor,
  });
  check(
    "new and resumed sessions return structured ACP cursors",
    sessionA.resumeCursor?.kind === "acp-v1" &&
      sessionB.resumeCursor?.kind === "acp-v1" &&
      JSON.stringify(resumed.resumeCursor) === JSON.stringify(resumeCursor),
  );
  check(
    "session identity includes generic package",
    JSON.stringify(sessionA.adapterPackage) === JSON.stringify(packageIdentity) &&
      sessionA.provider === "acpConformance",
  );

  const snapshot = yield* instance.snapshot.getSnapshot;
  const features = instance.adapter.capabilities.protocol.features;
  check(
    "snapshot identity and opaque model values",
    snapshot.driver === "acpConformance" &&
      snapshot.instanceId === "acp_conformance" &&
      JSON.stringify(snapshot.adapterPackage) === JSON.stringify(packageIdentity) &&
      snapshot.models.some(
        (model) => model.slug === "vendor/model@opaque:alpha" && model.isDefault === true,
      ) &&
      snapshot.models.some((model) => model.slug === secondModel),
  );
  check(
    "context occupancy is negotiated without token-breakdown usage",
    features.includes("stream.context") &&
      !features.includes("stream.usage") &&
      !features.includes("input.attachments") &&
      !features.includes("conversation.rollback") &&
      !features.includes("request.structured-input"),
  );

  const main = yield* complete(threadA, "canonical updates");
  const mainThread = yield* instance.adapter.readThread(threadA);
  const mainStored = mainThread.turns.find((turn) => turn.id === main.turn.turnId);
  const mainEvents = (mainStored?.items ?? []).filter((item) => item.type !== "userMessage");
  const mainTypes = mainEvents.map((event) => event.type);
  check(
    "canonical ACP event order",
    JSON.stringify(mainTypes) ===
      JSON.stringify([
        "turn.started",
        "item.started",
        "content.delta",
        "item.started",
        "item.completed",
        "thread.token-usage.updated",
        "item.started",
        "content.delta",
        "item.updated",
        "content.delta",
        "item.completed",
        "item.completed",
        "turn.completed",
      ]),
    JSON.stringify(mainTypes),
  );
  const thought = mainEvents.find(
    (event) => event.type === "content.delta" && event.payload.streamKind === "reasoning_text",
  );
  const toolCompleted = mainEvents.find(
    (event) => event.type === "item.completed" && event.itemId === "generic-tool-1",
  );
  const occupancy = mainEvents.find((event) => event.type === "thread.token-usage.updated");
  const imageFallback = mainEvents.find(
    (event) =>
      event.type === "content.delta" && event.payload.delta === "[Image: image/png; data:fixture]",
  );
  check(
    "canonical thought, generic tool, assistant, and non-text data",
    thought?.payload.delta === "considering" &&
      toolCompleted?.payload.itemType === "dynamic_tool_call" &&
      toolCompleted?.payload.data?.rawOutput?.answer === 42 &&
      mainEvents.some(
        (event) =>
          event.type === "content.delta" &&
          event.payload.streamKind === "assistant_text" &&
          event.payload.delta === "answer",
      ) &&
      imageFallback !== undefined,
  );
  check(
    "usage update retains context occupancy semantics",
    JSON.stringify(occupancy?.payload) ===
      JSON.stringify({ usage: { usedTokens: 37, maxTokens: 128 } }) &&
      occupancy?.payload.usage.inputTokens === undefined &&
      occupancy?.payload.usage.outputTokens === undefined,
  );
  check(
    "events carry bridge and package identity",
    mainEvents.every(
      (event) =>
        event.provider === "acpConformance" &&
        event.providerInstanceId === "acp_conformance" &&
        JSON.stringify(event.adapterPackage) === JSON.stringify(packageIdentity),
    ),
    JSON.stringify(
      mainEvents.map((event) => ({
        type: event.type,
        provider: event.provider,
        providerInstanceId: event.providerInstanceId,
        adapterPackage: event.adapterPackage,
      })),
    ),
  );

  const permissionEventsFiber = yield* collectFor(threadA);
  const openedFiber = yield* instance.adapter.streamEvents.pipe(
    Stream.filter((event) => event.threadId === threadA && event.type === "request.opened"),
    Stream.runHead,
    Effect.forkChild,
  );
  yield* Effect.yieldNow;
  const permissionTurn = yield* instance.adapter.sendTurn({
    threadId: threadA,
    input: "permission turn",
    attachments: [],
  });
  const openedOption = yield* Fiber.join(openedFiber);
  const opened = openedOption._tag === "Some" ? openedOption.value : undefined;
  check(
    "ACP permission opens a canonical approval",
    opened?.type === "request.opened" &&
      opened.payload.requestType === "command_execution_approval" &&
      opened.providerRefs?.providerItemId === "permission-tool",
  );
  yield* instance.adapter.respondToRequest(threadA, opened.requestId, "accept");
  const permissionEvents = yield* drain(permissionEventsFiber);
  check(
    "permission resolution completes its one turn",
    permissionEvents.filter(terminal).length === 1 &&
      permissionEvents.at(-1)?.turnId === permissionTurn.turnId &&
      permissionEvents.some(
        (event) => event.type === "request.resolved" && event.payload.decision === "accept",
      ),
  );

  const heldFiber = yield* collectFor(
    threadA,
    (event) => event.type === "content.delta" && event.payload.delta === "holding",
  );
  yield* Effect.yieldNow;
  const heldTurn = yield* instance.adapter.sendTurn({
    threadId: threadA,
    input: "hold for cancel",
    attachments: [],
  });
  const heldEvents = yield* drain(heldFiber);
  check("held prompt reaches its protocol milestone", !heldEvents.some(terminal));
  const cancelFiber = yield* collectFor(threadA);
  yield* Effect.yieldNow;
  yield* instance.adapter.interruptTurn(threadA, heldTurn.turnId);
  const cancelEvents = yield* drain(cancelFiber);
  check(
    "session/cancel produces exactly one cancelled terminal",
    cancelEvents.filter(terminal).length === 1 &&
      cancelEvents.at(-1)?.turnId === heldTurn.turnId &&
      cancelEvents.at(-1)?.payload.state === "cancelled" &&
      cancelEvents.at(-1)?.payload.stopReason === "cancelled",
  );

  const thread = yield* instance.adapter.readThread(threadA);
  check(
    "readThread retains each completed canonical turn",
    thread.turns.length === 3 &&
      thread.turns.every((turn) => turn.items.filter(terminal).length === 1),
  );

  yield* instance.adapter.stopSession(threadB);
  check(
    "session close removes one multiplexed session",
    !(yield* instance.adapter.hasSession(threadB)) &&
      (yield* instance.adapter.hasSession(threadA)) &&
      (yield* instance.adapter.hasSession(threadResumed)),
  );
  yield* instance.adapter.stopAll();
  check(
    "stopAll removes remaining sessions",
    !(yield* instance.adapter.hasSession(threadA)) &&
      !(yield* instance.adapter.hasSession(threadResumed)) &&
      (yield* instance.adapter.listSessions()).length === 0,
  );

  const audit = readAudit(adapterAuditPath);
  const requests = audit.filter((entry) => entry.kind === "request").map((entry) => entry.message);
  const methods = requests.map((message) => message.method).filter(Boolean);
  const configCalls = requests.filter((message) => message.method === "session/set_config_option");
  check(
    "one process initializes then multiplexes two new sessions and one resume",
    methods.filter((method) => method === "initialize").length === 1 &&
      methods.filter((method) => method === "session/new").length === 2 &&
      methods.filter((method) => method === "session/resume").length === 1,
    JSON.stringify(methods),
  );
  check(
    "resume is structured on the ACP wire",
    requests.some(
      (message) =>
        message.method === "session/resume" &&
        message.params.sessionId === resumeCursor.sessionId &&
        message.params.cwd === tempDirectory &&
        Array.isArray(message.params.mcpServers),
    ),
  );
  check(
    "model is sent before refreshed reasoning with opaque values intact",
    configCalls.length >= 2 &&
      configCalls[0].params.configId === "model" &&
      configCalls[0].params.value === secondModel &&
      configCalls[1].params.configId === "reasoning_effort" &&
      configCalls[1].params.value === secondReasoning,
    JSON.stringify(configCalls.map((message) => message.params)),
  );
  const permissionResolution = audit.find((entry) => entry.kind === "permission-resolution");
  check(
    "permission decision resolves the matching ACP option",
    JSON.stringify(permissionResolution?.result) ===
      JSON.stringify({ outcome: { outcome: "selected", optionId: "allow-once-wire" } }),
  );
  const cancelCount = methods.filter((method) => method === "session/cancel").length;
  const closeCount = methods.filter((method) => method === "session/close").length;
  const cleanEof = audit.some((entry) => entry.kind === "eof" && entry.trailingBytes === 0);
  check(
    "cancel, close, stopAll, and expected natural exit reach the wire",
    cancelCount === 1 && closeCount === 1 && cleanEof,
    JSON.stringify({ cancelCount, closeCount, cleanEof, tail: audit.slice(-4) }),
  );
}).pipe(Effect.scoped);

const runAcpConformance = program.pipe(
  Effect.ensuring(
    Effect.sync(() => {
      try {
        NodeFS.rmSync(tempDirectory, { recursive: true, force: true });
      } catch {}
    }),
  ),
  Effect.map(() => ({
    checks: result.checks,
    eventCount: result.allEvents.length,
  })),
);
export { runAcpConformance };
