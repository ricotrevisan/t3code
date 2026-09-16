/**
 * Pi RPC adapter conformance against a deterministic protocol fixture.
 * Prompt acknowledgement precedes asynchronous events. Collectors stop on
 * protocol milestones, not assumed delta counts. Real Pi needs a separate smoke.
 */
import * as NodeURL from "node:url";

import * as Effect from "effect/Effect";
import * as Fiber from "effect/Fiber";
import * as NodeServices from "@effect/platform-node/NodeServices";
import * as Stream from "effect/Stream";

const { makeExternalProviderDriver } = await import(
  new URL("../ExternalProviderDriver.ts", import.meta.url).href
);
const { makeExternalProviderProcessSupervisor } = await import(
  new URL("../ExternalProviderProcessSupervisor.ts", import.meta.url).href
);
const pkg = (await import(new URL("./piAdapterPackage.mjs", import.meta.url).href)).default;

const result = { checks: [], allEvents: [] };
const check = (name, ok, detail) => {
  result.checks.push({ name, ok, ...(detail === undefined ? {} : { detail }) });
};
const terminal = (event) => event.type === "turn.completed" || event.type === "turn.aborted";

const program = Effect.gen(function* () {
  const supervisor = yield* makeExternalProviderProcessSupervisor();
  const raw = yield* supervisor.processes.spawn({
    command: process.execPath,
    args: [NodeURL.fileURLToPath(new URL("./piHarness.mjs", import.meta.url))],
    purpose: { kind: "probe" },
  });
  const wireFiber = yield* raw.stdout.pipe(
    Stream.decodeText(),
    Stream.splitLines,
    Stream.map((line) => JSON.parse(line)),
    Stream.takeUntil((event) => event.type === "agent_settled"),
    Stream.runCollect,
    Effect.forkChild,
  );
  yield* raw.write(
    new TextEncoder().encode(
      `${JSON.stringify({ id: "wire", type: "prompt", message: "hello" })}\n`,
    ),
  );
  const wire = Array.from(yield* Fiber.join(wireFiber));
  check(
    "wire prompt ack precedes run events",
    wire[0]?.type === "response" &&
      wire[0]?.command === "prompt" &&
      wire[1]?.type === "agent_start",
  );
  yield* raw.close;
  check("fixture exits zero on stdin EOF", (yield* raw.exitCode) === 0);

  const driver = makeExternalProviderDriver(pkg, {});
  const instance = yield* driver.create({
    instanceId: "pi_ci",
    displayName: "Pi RPC",
    accentColor: undefined,
    environment: [],
    enabled: true,
    config: {},
  });
  const collectUntil = (predicate) =>
    instance.adapter.streamEvents.pipe(
      Stream.takeUntil(predicate),
      Stream.runCollect,
      Effect.forkChild,
    );
  const record = (events) => {
    result.allEvents.push(...events);
    return events;
  };
  const drain = (fiber) =>
    Fiber.join(fiber).pipe(Effect.map((events) => record(Array.from(events))));
  const send = (input, extra) =>
    instance.adapter.sendTurn({
      threadId: "pi-ci-thread",
      input,
      attachments: [],
      ...extra,
    });
  const complete = (input, extra) =>
    Effect.gen(function* () {
      const fiber = yield* collectUntil(terminal);
      const turn = yield* send(input, extra);
      const events = yield* drain(fiber);
      check(
        `one terminal: ${input}`,
        events.filter(terminal).length === 1 && events.at(-1)?.turnId === turn.turnId,
      );
      return events;
    });

  const snapshot = yield* instance.snapshot.getSnapshot;
  check(
    "probe discovers provider-qualified models",
    JSON.stringify(snapshot.models.map((model) => model.slug)) ===
      JSON.stringify(["anthropic/claude-sonnet-4", "openai/gpt-5.6", "custom/gpt-5.6"]),
  );
  check("manifest identity stamped", snapshot.driver === "piRpc");
  check(
    "unsupported capabilities omitted",
    !pkg.manifest.capabilities.includes("turn.follow-up") &&
      !pkg.manifest.capabilities.includes("stream.context"),
  );

  const session = yield* instance.adapter.startSession({
    threadId: "pi-ci-thread",
    providerInstanceId: "pi_ci",
    runtimeMode: "full-access",
    modelSelection: { instanceId: "pi_ci", model: "anthropic/claude-sonnet-4" },
  });
  check(
    "session ready",
    session.status === "ready" && typeof session.resumeCursor?.sessionFile === "string",
  );

  const plain = yield* complete("hello pi");
  check(
    "plain prompt events",
    JSON.stringify(plain.map((event) => event.type)) ===
      JSON.stringify([
        "turn.started",
        "content.delta",
        "content.delta",
        "thread.token-usage.updated",
        "turn.completed",
      ]),
  );
  check("reasoning delta", plain[1]?.payload.streamKind === "reasoning_text");
  check(
    "authoritative final usage",
    plain[3]?.payload.usage.usedTokens === 150 && plain[3]?.payload.usage.outputTokens === 30,
  );

  const heldFiber = yield* collectUntil((event) => event.type === "content.delta");
  const heldTurn = yield* send("!hold please");
  const held = yield* drain(heldFiber);
  check("held prompt remains active", held[0]?.type === "turn.started" && !held.some(terminal));
  const steerFiber = yield* collectUntil(terminal);
  const steeredTurn = yield* send("do this instead");
  const steered = yield* drain(steerFiber);
  check(
    "steer stays in one turn",
    steeredTurn.turnId === heldTurn.turnId && steered.at(-1)?.turnId === heldTurn.turnId,
  );

  const tool = yield* complete("!tool ls");
  check(
    "tool item lifecycle",
    tool.some((event) => event.type === "item.started" && event.itemId === "call_1") &&
      tool.some((event) => event.type === "item.completed" && event.itemId === "call_1"),
  );
  check(
    "tool loop finishes after final answer",
    tool.at(-2)?.type === "thread.token-usage.updated" &&
      tool.some(
        (event) => event.type === "content.delta" && event.payload.delta === "tool output handled",
      ) &&
      tool.filter((event) => event.type === "turn.started").length === 1,
  );

  const retry = yield* complete("!retry transient");
  check(
    "retry settles once as success",
    retry.at(-1)?.payload.state === "completed" &&
      retry.filter((event) => event.type === "turn.started").length === 1,
  );
  const failed = yield* complete("!error persistent");
  check(
    "provider failure is not success",
    failed.at(-1)?.payload.state === "failed" &&
      failed.at(-1)?.payload.errorMessage === "Inference failed",
  );

  for (const [method, answer, expected] of [
    ["select", "alpha", "alpha"],
    ["confirm", "Yes", "true"],
    ["input", "", ""],
    ["editor", "", ""],
  ]) {
    const askFiber = yield* collectUntil((event) => event.type === "user-input.requested");
    yield* send(`?${method}: pick`);
    const asked = yield* drain(askFiber);
    const requested = asked.at(-1);
    check(`ui request: ${method}`, requested?.type === "user-input.requested");
    let resolved = false;
    let completed = false;
    const doneFiber = yield* collectUntil((event) => {
      resolved ||= event.type === "user-input.resolved";
      completed ||= terminal(event);
      return resolved && completed;
    });
    yield* instance.adapter.respondToUserInput("pi-ci-thread", requested.requestId, {
      [requested.requestId]: answer,
    });
    const done = yield* drain(doneFiber);
    check(
      `ui response: ${method}`,
      done.some(
        (event) => event.type === "content.delta" && event.payload.delta === `chose: ${expected}`,
      ),
    );
  }
  const notifications = yield* complete("?notify display updates");
  check(
    "display updates do not create questions",
    !notifications.some((event) => event.type === "user-input.requested"),
  );

  const switched = yield* complete("switch model", {
    modelSelection: {
      instanceId: "pi_ci",
      model: "custom/gpt-5.6",
      options: [{ id: "reasoningEffort", value: "high" }],
    },
  });
  check("model selection preserves provider", switched.at(-1)?.payload.state === "completed");
  yield* complete("!branch keep active history");
  const thread = yield* instance.adapter.readThread("pi-ci-thread");
  check(
    "read thread filters metadata and sibling branches",
    thread.turns.length > 5 &&
      thread.turns.every((turn) => turn.id !== "off-branch" && turn.items[0]?.role !== undefined) &&
      thread.turns.some((turn) => turn.items[0]?.role === "assistant"),
  );

  const rejected = yield* send("!reject preflight").pipe(Effect.result);
  check("preflight rejection reported", rejected._tag === "Failure");
  const afterRejected = yield* complete("after rejection");
  check(
    "rejected prompt does not poison next turn",
    afterRejected.some((event) => event.payload.delta === "echo: after rejection"),
  );
  const handled = yield* complete("!handled extension command");
  check(
    "handled non-agent prompt completes explicitly",
    JSON.stringify(handled.map((event) => event.type)) ===
      JSON.stringify(["turn.started", "turn.completed"]),
  );
  yield* complete("after handled prompt");

  const abortHoldFiber = yield* collectUntil((event) => event.type === "content.delta");
  yield* send("!hold forever");
  yield* drain(abortHoldFiber);
  const abortFiber = yield* collectUntil(terminal);
  yield* instance.adapter.interruptTurn("pi-ci-thread");
  const aborted = yield* drain(abortFiber);
  check("abort maps to settled aborted", aborted.at(-1)?.type === "turn.aborted");
  const afterAbort = yield* complete("after abort");
  check(
    "new prompt works after abort",
    afterAbort.some((event) => event.payload.delta === "echo: after abort"),
  );

  const cancelledResume = yield* instance.adapter
    .startSession({
      threadId: "pi-cancelled",
      runtimeMode: "full-access",
      resumeCursor: { sessionFile: "/cancelled.jsonl" },
    })
    .pipe(Effect.result);
  check(
    "resume cancellation rejected",
    cancelledResume._tag === "Failure" && !(yield* instance.adapter.hasSession("pi-cancelled")),
  );
  const invalidModelStart = yield* instance.adapter
    .startSession({
      threadId: "pi-invalid-model",
      runtimeMode: "full-access",
      modelSelection: { instanceId: "pi_ci", model: "missing/model" },
    })
    .pipe(Effect.result);
  check(
    "failed initialization leaves no session",
    invalidModelStart._tag === "Failure" &&
      !(yield* instance.adapter.hasSession("pi-invalid-model")),
  );
  yield* instance.adapter.startSession({
    threadId: "pi-invalid-model",
    runtimeMode: "full-access",
  });
  yield* instance.adapter.stopSession("pi-invalid-model");

  const rollback = yield* instance.adapter.rollbackThread("pi-ci-thread", 1).pipe(Effect.result);
  check("rollback not negotiated", rollback._tag === "Failure");

  yield* instance.adapter.startSession({ threadId: "pi-exit", runtimeMode: "full-access" });
  const exitFiber = yield* collectUntil(
    (event) => event.type === "runtime.error" && event.threadId === "pi-exit",
  );
  const exited = yield* instance.adapter
    .sendTurn({ threadId: "pi-exit", input: "!exit" })
    .pipe(Effect.result);
  yield* drain(exitFiber);
  check("EOF releases pending RPC", exited._tag === "Failure");
  yield* instance.adapter.stopSession("pi-exit");

  yield* instance.adapter.stopSession("pi-ci-thread");
  check("session removed after stop", !(yield* instance.adapter.hasSession("pi-ci-thread")));
}).pipe(Effect.scoped);

await Effect.runPromise(program.pipe(Effect.provide(NodeServices.layer))).then(
  () => {
    result.eventCount = result.allEvents.length;
    delete result.allEvents;
    result.ok = result.checks.every((entry) => entry.ok);
  },
  (error) => {
    result.ok = false;
    result.error = String(error?.message ?? error);
    result.stack = String(error?.stack ?? "").slice(0, 2000);
  },
);
console.log(JSON.stringify(result));
