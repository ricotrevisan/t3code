import * as NodeURL from "node:url";
import * as Effect from "effect/Effect";
import * as Fiber from "effect/Fiber";
import * as FileSystem from "effect/FileSystem";
import * as Queue from "effect/Queue";
import * as Stream from "effect/Stream";
import { PI_PROVIDER_ADAPTER_PACKAGE as pkg } from "@t3tools/provider-adapter-pi";
import { ServerConfig } from "../../config.ts";
import { resolveAttachmentPath } from "../../attachmentStore.ts";
import { makeExternalProviderDriver } from "../ExternalProviderDriver.ts";
import { makeExternalProviderProcessSupervisor } from "../ExternalProviderProcessSupervisor.ts";
import { runtimeEventToActivities } from "../../orchestration/Layers/ProviderRuntimeIngestion.ts";
import { foldSubagentActivities } from "../../../../../packages/client-runtime/src/state/subagentRuntime.ts";
const result = { checks: [], allEvents: [] };
const check = (name, ok, detail) => {
  result.checks.push({ name, ok, ...(detail === void 0 ? {} : { detail }) });
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
      `${JSON.stringify({ id: "wire", type: "prompt", message: "hello" })}
`,
    ),
  );
  const wire = Array.from(yield* Fiber.join(wireFiber));
  check(
    "wire prompt ack precedes run events",
    wire[0]?.type === "response" &&
      wire[0]?.command === "prompt" &&
      wire.findIndex((event) => event.type === "agent_start") > 0,
  );
  yield* raw.close;
  check("fixture exits zero on stdin EOF", (yield* raw.exitCode) === 0);
  const driver = makeExternalProviderDriver(pkg, {
    binaryPath: process.execPath,
    args: [NodeURL.fileURLToPath(new URL("./piHarness.mjs", import.meta.url))],
  });
  const unsafeFlags = [
    "--",
    "--help",
    "-h",
    "--version",
    "-v",
    "--export",
    "--list-models",
    "--mode",
    "--mode=rpc",
    "--session-dir=/tmp/outside",
    "--session",
    "--session-id=outside",
    "--fork",
    "--continue",
    "--resume",
    "--no-session",
    "-c",
    "-r=outside",
  ];
  const unsafeResults = yield* Effect.forEach(unsafeFlags, (unsafeFlag, index) => {
    const unsafeConfig = {
      binaryPath: process.execPath,
      args: [NodeURL.fileURLToPath(new URL("./piHarness.mjs", import.meta.url)), unsafeFlag],
    };
    return makeExternalProviderDriver(pkg, unsafeConfig)
      .create({
        instanceId: `pi_unsafe_${index}`,
        displayName: "Pi",
        accentColor: void 0,
        environment: [],
        enabled: true,
        config: unsafeConfig,
      })
      .pipe(Effect.result);
  });
  check(
    "unsafe Pi session flags are rejected",
    unsafeResults.every((result) => result._tag === "Failure"),
  );

  const instance = yield* driver.create({
    instanceId: "pi_ci",
    displayName: "Pi RPC",
    accentColor: void 0,
    environment: [],
    enabled: true,
    config: {
      binaryPath: process.execPath,
      args: [NodeURL.fileURLToPath(new URL("./piHarness.mjs", import.meta.url))],
    },
  });
  const eventQueue = yield* instance.adapter.streamEvents.pipe(
    Stream.toQueue({ capacity: "unbounded" }),
  );
  const takeUntil = (queue, predicate) =>
    Effect.gen(function* () {
      const events = [];
      for (;;) {
        const event = yield* Queue.take(queue);
        events.push(event);
        if (predicate(event)) return events;
      }
    });
  const collectUntil = (predicate) => takeUntil(eventQueue, predicate).pipe(Effect.forkChild);
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
  const anthropicReasoning = snapshot.models.find(
    (model) => model.slug === "anthropic/claude-sonnet-4",
  )?.capabilities.optionDescriptors?.[0];
  const customReasoning = snapshot.models.find((model) => model.slug === "custom/gpt-5.6")
    ?.capabilities.optionDescriptors?.[0];
  check(
    "reasoning levels are discovered per model",
    anthropicReasoning?.type === "select" &&
      anthropicReasoning.options.some((option) => option.id === "high") &&
      customReasoning?.type === "select" &&
      !customReasoning.options.some((option) => option.id === "high"),
  );
  check("manifest identity stamped", snapshot.driver === "piRpc");
  check(
    "only full-access runtime mode is advertised",
    snapshot.supportedRuntimeModes?.length === 1 &&
      snapshot.supportedRuntimeModes[0] === "full-access",
  );
  const restrictedStart = yield* instance.adapter
    .startSession({
      threadId: "pi-restricted",
      providerInstanceId: "pi_ci",
      runtimeMode: "approval-required",
    })
    .pipe(Effect.result);
  check("non-full-access startup is rejected", restrictedStart._tag === "Failure");
  check(
    "unsupported capabilities omitted",
    !pkg.manifest.capabilities.includes("turn.follow-up") &&
      !pkg.manifest.capabilities.includes("stream.context"),
  );
  const concurrentSessions = yield* Effect.all(
    [
      instance.adapter.startSession({ threadId: "pi-concurrent", runtimeMode: "full-access" }),
      instance.adapter.startSession({ threadId: "pi-concurrent", runtimeMode: "full-access" }),
    ],
    { concurrency: "unbounded" },
  );
  check(
    "concurrent starts share one native session",
    concurrentSessions[0]?.resumeCursor?.sessionId ===
      concurrentSessions[1]?.resumeCursor?.sessionId,
  );
  yield* instance.adapter.stopSession("pi-concurrent");

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
  const attachmentTurn = yield* instance.adapter
    .sendTurn({
      threadId: "pi-ci-thread",
      input: "do not discard this",
      attachments: [
        { type: "file", id: "not-read", name: "notes.txt", mimeType: "text/plain", sizeBytes: 1 },
      ],
    })
    .pipe(Effect.result);
  check("unsupported attachments are rejected", attachmentTurn._tag === "Failure");
  const emptyTurn = yield* instance.adapter
    .sendTurn({
      threadId: "pi-ci-thread",
      input: "   ",
      attachments: [],
    })
    .pipe(Effect.result);
  check("empty prompts are rejected", emptyTurn._tag === "Failure");
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
  check("image attachments advertised", pkg.manifest.capabilities.includes("input.attachments"));
  const fileSystem = yield* FileSystem.FileSystem;
  const config = yield* ServerConfig;
  const imageData =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aMioAAAAASUVORK5CYII=";
  const bytes = Buffer.from(imageData, "base64");
  const expectedImages = [
    { type: "image", data: imageData, mimeType: "image/png" },
    {
      type: "image",
      data: "R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7",
      mimeType: "image/gif",
    },
  ];
  const images = expectedImages.map((image, index) => ({
    type: "image",
    id: `pi-image-${index}`,
    name: `image.${image.mimeType.split("/")[1]}`,
    mimeType: image.mimeType,
    sizeBytes: Buffer.from(image.data, "base64").length,
  }));
  yield* fileSystem.makeDirectory(config.attachmentsDir, { recursive: true });
  for (const [index, attachment] of images.entries()) {
    yield* fileSystem.writeFile(
      resolveAttachmentPath({ attachmentsDir: config.attachmentsDir, attachment }),
      Buffer.from(expectedImages[index].data, "base64"),
    );
  }
  const checkImageMessage = (message, expected = expectedImages) =>
    Effect.gen(function* () {
      const thread = yield* instance.adapter.readThread("pi-ci-thread");
      const userMessage = thread.turns
        .flatMap((turn) => turn.items)
        .findLast((item) => item.role === "user");
      check(
        `image content reaches Pi: ${message || "image-only"}`,
        JSON.stringify(userMessage?.content) ===
          JSON.stringify([{ type: "text", text: message }, ...expected]),
      );
    });
  for (const message of ["describe these images", undefined]) {
    yield* complete(message, { attachments: images });
    yield* checkImageMessage(message ?? "");
  }
  for (const message of ["look at this instead", undefined]) {
    const started = yield* collectUntil((event) => event.type === "content.delta");
    const active = yield* send("!hold image steering");
    yield* drain(started);
    const completed = yield* collectUntil(terminal);
    const steered = yield* send(message, { attachments: images });
    const events = yield* drain(completed);
    check(
      "image steering retains active turn",
      steered.turnId === active.turnId && events.at(-1)?.turnId === active.turnId,
    );
    yield* checkImageMessage(message ?? "");
  }
  // Pi echoes user images in its events; a normal upload can exceed the old 2 MB RPC line cap.
  const largeBytes = Buffer.concat([bytes, Buffer.alloc(1_600_000)]);
  const largeImage = { ...images[0], id: "pi-large", sizeBytes: largeBytes.length };
  yield* fileSystem.writeFile(
    resolveAttachmentPath({ attachmentsDir: config.attachmentsDir, attachment: largeImage }),
    largeBytes,
  );
  yield* complete("large image", { attachments: [largeImage] });
  yield* checkImageMessage("large image", [
    { type: "image", data: largeBytes.toString("base64"), mimeType: "image/png" },
  ]);

  const beforeFailure = yield* instance.adapter.readThread("pi-ci-thread");
  const missingImage = yield* send("missing image", {
    attachments: [{ ...images[0], id: "pi-missing" }],
  }).pipe(Effect.result);
  const afterFailure = yield* instance.adapter.readThread("pi-ci-thread");
  check(
    "unreadable images fail without sending a prompt",
    missingImage._tag === "Failure" &&
      missingImage.failure.message.includes("attachment file does not exist") &&
      JSON.stringify(beforeFailure) === JSON.stringify(afterFailure),
  );
  const agentActivities = [];
  for (let run = 1; run <= 2; run++) {
    const children = yield* complete("!subagents");
    agentActivities.push(...children.flatMap((event) => runtimeEventToActivities(event)));
    const roster = foldSubagentActivities(agentActivities);
    check(
      "Pi agents reach the client roster",
      roster.length === 2 &&
        roster[0].title === "baseline" &&
        roster[0].status === "completed" &&
        roster[1].status === "failed",
    );
    check(
      "Pi session reactivation updates the existing row",
      roster[0]?.activationCount === run && roster[0]?.usage?.totalTokens === run * 20,
    );
    const starts = children.filter((event) => event.type === "task.started");
    const progress = children.filter((event) => event.type === "task.progress");
    const ends = children.filter((event) => event.type === "task.completed");
    check(
      "Pi parallel children have stable identities",
      starts.length === 2 && starts[0].payload.taskId === "pi:child-baseline",
    );
    check("Pi heartbeat is deduplicated", progress.length === 2);
    check(
      "Pi children settle independently",
      ends.length === 2 &&
        ends[0].payload.status === "completed" &&
        ends[1].payload.status === "failed",
    );
    check(
      "Pi usage accumulates across reused sessions",
      ends[0]?.payload.typedUsage?.totalTokens === run * 20,
    );
  }
  const tool = yield* complete("!tool ls");
  check(
    "tool item lifecycle",
    tool.some((event) => event.type === "item.started" && event.itemId === "call_1") &&
      tool.some((event) => event.type === "item.updated" && event.itemId === "call_1") &&
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
      options: [{ id: "reasoningEffort", value: "low" }],
    },
  });
  check("model selection preserves provider", switched.at(-1)?.payload.state === "completed");
  yield* complete("!branch keep active history");
  const thread = yield* instance.adapter.readThread("pi-ci-thread");
  check(
    "read thread filters metadata and sibling branches",
    thread.turns.length > 5 &&
      thread.turns.every((turn) => turn.id !== "off-branch" && turn.items[0]?.role !== void 0) &&
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
  const crashedSession = yield* instance.adapter.startSession({
    threadId: "pi-exit",
    runtimeMode: "full-access",
  });
  const exitFiber = yield* collectUntil(
    (event) => event.type === "runtime.error" && event.threadId === "pi-exit",
  );
  const exited = yield* instance.adapter
    .sendTurn({ threadId: "pi-exit", input: "!exit" })
    .pipe(Effect.result);
  yield* drain(exitFiber);
  check("EOF releases pending RPC", exited._tag === "Failure");
  check("crashed sessions are removed", !(yield* instance.adapter.hasSession("pi-exit")));
  const recoveredSession = yield* instance.adapter.startSession({
    threadId: "pi-exit",
    runtimeMode: "full-access",
    resumeCursor: crashedSession.resumeCursor,
  });
  check(
    "crashed sessions can resume on a new process",
    recoveredSession.resumeCursor?.sessionId === crashedSession.resumeCursor?.sessionId,
  );
  yield* instance.adapter.stopSession("pi-exit");
  const resumeCursor = session.resumeCursor;
  yield* instance.adapter.stopSession("pi-ci-thread");
  check("session removed after stop", !(yield* instance.adapter.hasSession("pi-ci-thread")));
  const restarted = yield* driver.create({
    instanceId: "pi_ci",
    displayName: "Pi",
    accentColor: void 0,
    environment: [],
    enabled: true,
    config: {
      binaryPath: process.execPath,
      args: [NodeURL.fileURLToPath(new URL("./piHarness.mjs", import.meta.url))],
    },
  });
  const resumed = yield* restarted.adapter.startSession({
    threadId: "pi-ci-thread",
    providerInstanceId: "pi_ci",
    runtimeMode: "full-access",
    resumeCursor,
  });
  check(
    "resume survives adapter and process restart",
    resumed.resumeCursor?.sessionFile === resumeCursor?.sessionFile &&
      resumed.resumeCursor?.sessionId === resumeCursor?.sessionId,
  );
  const restartedEventQueue = yield* restarted.adapter.streamEvents.pipe(
    Stream.toQueue({ capacity: "unbounded" }),
  );
  const resumedFiber = yield* takeUntil(restartedEventQueue, terminal).pipe(Effect.forkChild);
  const resumedTurn = yield* restarted.adapter.sendTurn({
    threadId: "pi-ci-thread",
    input: "after native resume",
    attachments: [],
  });
  const resumedEvents = Array.from(yield* Fiber.join(resumedFiber));
  result.allEvents.push(...resumedEvents);
  check(
    "new prompt completes after native resume",
    resumedEvents.some(
      (event) =>
        event.type === "content.delta" && event.payload.delta === "echo: after native resume",
    ) &&
      resumedEvents.at(-1)?.type === "turn.completed" &&
      resumedEvents.at(-1)?.turnId === resumedTurn.turnId,
  );
  yield* restarted.adapter.stopSession("pi-ci-thread");
}).pipe(Effect.scoped);
const runPiConformance = program.pipe(
  Effect.map(() => ({
    checks: result.checks,
    eventCount: result.allEvents.length,
  })),
);
export { runPiConformance };
