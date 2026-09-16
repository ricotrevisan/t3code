/**
 * Pi adapter conformance scenario runner.
 *
 * Executed as a child Node process by PiAdapterConformance.test.ts so the
 * package runs in a single Effect module graph (production shape). Each
 * phase forks an exact-count event collector and joins it after the
 * triggering command resolves; Pi emits all events for a command before its
 * response, and the adapter queues them before resolving, so joins are
 * deterministic. Prints one JSON result object to stdout.
 */
import * as Effect from "effect/Effect";
import * as Fiber from "effect/Fiber";
import * as NodeServices from "@effect/platform-node/NodeServices";
import * as Stream from "effect/Stream";

const { makeExternalProviderDriver } = await import(
  new URL("../ExternalProviderDriver.ts", import.meta.url).href
);
const pkg = (await import(new URL("./piAdapterPackage.mjs", import.meta.url).href)).default;

const result = { checks: [], allEvents: [] };
const check = (name, ok, detail) => {
  result.checks.push({ name, ok, ...(detail === undefined ? {} : { detail }) });
};

const program = Effect.gen(function* () {
  const driver = makeExternalProviderDriver(pkg, {});
  const instance = yield* driver.create({
    instanceId: "pi_ci",
    displayName: "Pi RPC",
    accentColor: undefined,
    environment: [],
    enabled: true,
    config: {},
  });

  /** Fork an exact-count collector from the (queue-backed) event stream. */
  const takeEvents = (count) =>
    instance.adapter.streamEvents.pipe(Stream.take(count), Stream.runCollect, Effect.forkChild);
  const drain = (forkEffect) =>
    Effect.gen(function* () {
      const fiber = yield* forkEffect;
      return Array.from(yield* Fiber.join(fiber));
    });
  const record = (events) => {
    result.allEvents.push(
      ...events.map((event) => ({
        type: event.type,
        turnId: event.turnId,
        payload: event.payload,
        itemId: event.itemId,
      })),
    );
    return events;
  };
  const send = (input, extra) =>
    instance.adapter.sendTurn({
      threadId: "pi-ci-thread",
      input,
      attachments: [],
      ...(extra === undefined ? {} : extra),
    });

  // Model discovery through a short-lived probe process.
  const snapshot = yield* instance.snapshot.getSnapshot;
  check(
    "probe discovers models",
    snapshot.models.length === 2 && snapshot.models[1]?.slug === "gpt-5.6",
  );
  check("manifest identity stamped", snapshot.driver === "piRpc");

  const session = yield* instance.adapter.startSession({
    threadId: "pi-ci-thread",
    providerInstanceId: "pi_ci",
    runtimeMode: "full-access",
  });
  check(
    "session ready",
    session.status === "ready" && typeof session.resumeCursor?.sessionFile === "string",
  );

  // Plain prompt: turn lifecycle, reasoning, assistant text, usage.
  const plainFiber = takeEvents(5);
  const turn = yield* send("hello pi");
  const plain = record(yield* drain(plainFiber));
  check("turn id minted", String(turn.turnId).startsWith("pi-turn-"));
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
    JSON.stringify(plain.map((event) => event.type)),
  );
  check(
    "reasoning delta",
    plain[1]?.payload.streamKind === "reasoning_text" && plain[1]?.payload.delta === "thinking...",
  );
  check(
    "assistant delta bound to turn",
    plain[2]?.payload.delta === "echo: hello pi" && plain[2]?.turnId === turn.turnId,
  );
  check("usage sampled", plain[3]?.payload.usage.usedTokens === 150);

  // Mid-turn steering keeps one canonical turn.
  const heldFiber = takeEvents(3);
  const heldTurn = yield* send("!hold please");
  const held = record(yield* drain(heldFiber));
  check(
    "held prompt events",
    JSON.stringify(held.map((event) => event.type)) ===
      JSON.stringify(["turn.started", "content.delta", "thread.token-usage.updated"]),
    JSON.stringify(held.map((event) => event.type)),
  );
  const steerFiber = takeEvents(3);
  const steeredTurn = yield* send("do this instead");
  const steered = record(yield* drain(steerFiber));
  check("steer stays in one turn", steeredTurn.turnId === heldTurn.turnId);
  check(
    "steer completes the held turn",
    steered[0]?.payload.delta === "steered: do this instead" &&
      steered[2]?.type === "turn.completed",
    JSON.stringify(steered.map((event) => event.type)),
  );

  // Tool lifecycle.
  const toolFiber = takeEvents(7);
  yield* send("!tool ls");
  const tool = record(yield* drain(toolFiber));
  check(
    "tool item lifecycle",
    tool[1]?.type === "item.started" &&
      tool[1]?.itemId === "call_1" &&
      tool[1]?.payload.itemType === "command_execution" &&
      tool[2]?.type === "item.completed" &&
      tool[2]?.payload.status === "completed",
    JSON.stringify(tool.map((event) => `${event.type}:${event.itemId ?? "-"}`)),
  );

  // Structured input through the extension-UI sub-protocol.
  const uiAskFiber = takeEvents(1);
  yield* send("?select: pick");
  const requested = record(yield* drain(uiAskFiber))[0];
  check(
    "ui request surfaced",
    requested?.type === "user-input.requested" &&
      typeof requested.requestId === "string" &&
      requested.payload.questions[0]?.options.length === 2,
    JSON.stringify(requested),
  );
  const uiDoneFiber = takeEvents(5);
  yield* instance.adapter.respondToUserInput("pi-ci-thread", requested.requestId, {
    [requested.requestId]: "alpha",
  });
  const uiDone = record(yield* drain(uiDoneFiber));
  check(
    "ui response resolved",
    uiDone.some((event) => event.type === "user-input.resolved") &&
      uiDone.some((event) => event.type === "turn.started") &&
      uiDone.some(
        (event) => event.type === "content.delta" && event.payload.delta === "chose: alpha",
      ) &&
      uiDone.some((event) => event.type === "turn.completed"),
    JSON.stringify(uiDone.map((event) => event.type)),
  );

  // Model + reasoning selection apply before the next prompt.
  const switchFiber = takeEvents(5);
  yield* send("switch model", {
    modelSelection: {
      instanceId: "pi_ci",
      model: "gpt-5.6",
      options: [{ id: "reasoningEffort", value: "high" }],
    },
  });
  const switched = record(yield* drain(switchFiber));
  check(
    "model switch turn completed",
    JSON.stringify(switched.map((event) => event.type)) ===
      JSON.stringify([
        "turn.started",
        "content.delta",
        "content.delta",
        "thread.token-usage.updated",
        "turn.completed",
      ]),
    JSON.stringify(switched.map((event) => event.type)),
  );

  // Read thread through the durable entry cursor.
  const thread = yield* instance.adapter.readThread("pi-ci-thread");
  check(
    "read thread entries",
    thread.turns.length >= 5 &&
      thread.turns[0]?.id === "entry-1" &&
      thread.turns[1]?.id === "entry-2",
    JSON.stringify(thread.turns.map((turn) => turn.id)),
  );

  // Rollback advertised unsupported, not silently emulated.
  const rollback = yield* instance.adapter.rollbackThread("pi-ci-thread", 1).pipe(Effect.result);
  check("rollback not negotiated", rollback._tag === "Failure");

  // Abort a held turn maps to the canonical aborted event.
  const abortHoldFiber = takeEvents(3);
  yield* send("!hold forever");
  record(yield* drain(abortHoldFiber));
  const abortFiber = takeEvents(1);
  yield* instance.adapter.interruptTurn("pi-ci-thread");
  const aborted = record(yield* drain(abortFiber));
  check(
    "abort maps to canonical aborted",
    aborted[0]?.type === "turn.aborted" && aborted[0]?.payload.reason === "aborted by user",
  );

  // Graceful stop: host closes stdin, Pi exits 0, and no host-generated
  // runtime.error appears for the intentional shutdown.
  yield* instance.adapter.stopSession("pi-ci-thread");
  const hasSession = yield* instance.adapter.hasSession("pi-ci-thread");
  check(
    "graceful stop without crash events",
    hasSession === false && result.allEvents.every((event) => event.type !== "runtime.error"),
  );
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
