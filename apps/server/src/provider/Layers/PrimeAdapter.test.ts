// @effect-diagnostics nodeBuiltinImport:off
import * as NodeFSP from "node:fs/promises";
import * as NodeOS from "node:os";
import * as NodePath from "node:path";
import * as NodeURL from "node:url";

import * as NodeServices from "@effect/platform-node/NodeServices";
import { assert, it } from "@effect/vitest";
import {
  ApprovalRequestId,
  PrimeSettings,
  ProviderDriverKind,
  ProviderInstanceId,
  ThreadId,
  type ProviderRuntimeEvent,
} from "@t3tools/contracts";
import { createModelSelection } from "@t3tools/shared/model";
import * as Clock from "effect/Clock";
import * as Deferred from "effect/Deferred";
import * as Effect from "effect/Effect";
import * as Fiber from "effect/Fiber";
import * as FileSystem from "effect/FileSystem";
import * as Layer from "effect/Layer";
import * as Option from "effect/Option";
import * as Schema from "effect/Schema";
import * as Stream from "effect/Stream";

import { ServerConfig } from "../../config.ts";
import { makePrimeAdapter } from "./PrimeAdapter.ts";

const decodePrimeSettings = Schema.decodeSync(PrimeSettings);
const decodeRequestLog = Schema.decodeUnknownSync(
  Schema.fromJsonString(
    Schema.Struct({
      args: Schema.Array(Schema.String),
      command: Schema.Struct({
        type: Schema.String,
        id: Schema.optional(Schema.String),
        provider: Schema.optional(Schema.String),
        modelId: Schema.optional(Schema.String),
        level: Schema.optional(Schema.String),
        confirmed: Schema.optional(Schema.Boolean),
        cancelled: Schema.optional(Schema.Boolean),
        message: Schema.optional(Schema.String),
        value: Schema.optional(Schema.String),
      }),
    }),
  ),
);
const __dirname = NodePath.dirname(NodeURL.fileURLToPath(import.meta.url));
const mockAgentPath = NodePath.join(__dirname, "../../scripts/prime-rpc-mock-agent.ts");

const primeAdapterTestLayer = ServerConfig.layerTest(process.cwd(), {
  prefix: "t3code-prime-adapter-test-",
}).pipe(Layer.provideMerge(NodeServices.layer));

async function makeMockPrimeWrapper(extraEnv?: Record<string, string>): Promise<string> {
  const directory = await NodeFSP.mkdtemp(NodePath.join(NodeOS.tmpdir(), "prime-rpc-mock-"));
  const wrapperPath = NodePath.join(directory, "prime-agent");
  const envExports = Object.entries(extraEnv ?? {})
    .map(([key, value]) => `export ${key}=${JSON.stringify(value)}`)
    .join("\n");
  await NodeFSP.writeFile(
    wrapperPath,
    `#!/bin/sh
${envExports}
exec ${JSON.stringify(process.execPath)} ${JSON.stringify(mockAgentPath)} "$@"
`,
    "utf8",
  );
  await NodeFSP.chmod(wrapperPath, 0o755);
  return wrapperPath;
}

const makeTestAdapter = (
  binaryPath: string,
  options?: Parameters<typeof makePrimeAdapter>[1] & { readonly launchArgs?: string },
) =>
  makePrimeAdapter(
    decodePrimeSettings({
      enabled: true,
      binaryPath,
      ...(options?.launchArgs !== undefined ? { launchArgs: options.launchArgs } : {}),
    }),
    options,
  );

function waitForFileContent(
  filePath: string,
  attempts = 40,
  expectedContent?: string,
): Effect.Effect<string> {
  const readAttempt = (remainingAttempts: number): Effect.Effect<string> =>
    Effect.gen(function* () {
      if (remainingAttempts <= 0) {
        return yield* Effect.die(new Error(`Timed out waiting for file content at ${filePath}`));
      }
      const raw = yield* Effect.tryPromise(() => NodeFSP.readFile(filePath, "utf8")).pipe(
        Effect.orElseSucceed(() => ""),
      );
      if (
        raw.trim().length > 0 &&
        (expectedContent === undefined || raw.includes(expectedContent))
      ) {
        return raw;
      }
      yield* Effect.sleep("25 millis");
      return yield* readAttempt(remainingAttempts - 1);
    });
  return readAttempt(attempts);
}

it.layer(primeAdapterTestLayer, { excludeTestServices: true })("PrimeAdapter", (it) => {
  it.effect("maps a Prime confirm request to T3 approval and resumes after accept", () =>
    Effect.gen(function* () {
      const config = yield* ServerConfig;
      const requestLogPath = NodePath.join(config.baseDir, "prime-approval-requests.ndjson");
      const sideEffectPath = NodePath.join(config.baseDir, "prime-approval-side-effect.txt");
      const binaryPath = yield* Effect.promise(() =>
        makeMockPrimeWrapper({
          T3_PRIME_MOCK_REQUEST_LOG_PATH: requestLogPath,
          T3_PRIME_MOCK_APPROVAL_SIDE_EFFECT_PATH: sideEffectPath,
        }),
      );
      const adapter = yield* makeTestAdapter(binaryPath, {
        instanceId: ProviderInstanceId.make("primeAgent"),
      });
      const threadId = ThreadId.make("prime-approval-thread");
      const approvalOpened = yield* Deferred.make<ProviderRuntimeEvent>();
      const completed = yield* Deferred.make<void>();
      const eventFiber = yield* adapter.streamEvents.pipe(
        Stream.runForEach((event) =>
          Effect.gen(function* () {
            if (event.type === "request.opened") {
              yield* Deferred.succeed(approvalOpened, event);
            }
            if (event.type === "turn.completed") {
              yield* Deferred.succeed(completed, undefined);
            }
          }),
        ),
        Effect.forkChild,
      );

      yield* adapter.startSession({
        threadId,
        provider: ProviderDriverKind.make("primeAgent"),
        providerInstanceId: ProviderInstanceId.make("primeAgent"),
        cwd: process.cwd(),
        runtimeMode: "approval-required",
      });
      const turn = yield* adapter.sendTurn({
        threadId,
        input: "approval side effect",
        attachments: [],
      });
      const requested = yield* Deferred.await(approvalOpened);

      assert.equal(requested.type, "request.opened");
      if (requested.type !== "request.opened") {
        return;
      }
      assert.equal(requested.requestId, "prime-approval-1");
      assert.equal(requested.turnId, turn.turnId);
      assert.equal(requested.payload.requestType, "command_execution_approval");
      assert.include(requested.payload.detail ?? "", "ipython");
      assert.deepEqual(requested.providerRefs, { providerRequestId: "prime-approval-1" });
      assert.isFalse(
        yield* Effect.promise(() =>
          NodeFSP.stat(sideEffectPath).then(
            () => true,
            () => false,
          ),
        ),
      );

      yield* adapter.respondToRequest(
        threadId,
        ApprovalRequestId.make("prime-approval-1"),
        "accept",
      );
      yield* Deferred.await(completed);

      const recorded = (yield* Effect.promise(() => NodeFSP.readFile(requestLogPath, "utf8")))
        .trim()
        .split("\n")
        .map((line) => decodeRequestLog(line));
      const handshake = recorded.find((entry) => entry.command.type === "get_commands");
      assert.isDefined(handshake);
      assert.include(handshake?.args ?? [], "--no-extensions");
      const extensionPaths = (handshake?.args ?? []).flatMap((arg, index, args) =>
        arg === "--extension" && args[index + 1] ? [args[index + 1]!] : [],
      );
      assert.isTrue(extensionPaths.some((path) => path.endsWith("t3-approval-v1.ts")));
      assert.isTrue(extensionPaths.some((path) => path.endsWith("t3-openrouter-catalog.ts")));
      assert.include(handshake?.args ?? [], "--t3-approval-mode=approval-required");

      const response = recorded.find((entry) => entry.command.type === "extension_ui_response");
      assert.deepEqual(response?.command, {
        type: "extension_ui_response",
        id: "prime-approval-1",
        confirmed: true,
      });
      assert.equal(
        yield* Effect.promise(() => NodeFSP.readFile(sideEffectPath, "utf8")),
        "approved\n",
      );

      yield* adapter.stopSession(threadId);
      yield* Fiber.interrupt(eventFiber);
    }),
  );

  it.effect("rejects a Prime approval without running the tool side effect", () =>
    Effect.gen(function* () {
      const config = yield* ServerConfig;
      const requestLogPath = NodePath.join(config.baseDir, "prime-decline-requests.ndjson");
      const sideEffectPath = NodePath.join(config.baseDir, "prime-decline-side-effect.txt");
      const binaryPath = yield* Effect.promise(() =>
        makeMockPrimeWrapper({
          T3_PRIME_MOCK_REQUEST_LOG_PATH: requestLogPath,
          T3_PRIME_MOCK_APPROVAL_SIDE_EFFECT_PATH: sideEffectPath,
        }),
      );
      const adapter = yield* makeTestAdapter(binaryPath, {
        instanceId: ProviderInstanceId.make("primeAgent"),
      });
      const threadId = ThreadId.make("prime-decline-thread");
      const approvalOpened = yield* Deferred.make<void>();
      const completed = yield* Deferred.make<void>();
      const eventFiber = yield* adapter.streamEvents.pipe(
        Stream.runForEach((event) =>
          Effect.gen(function* () {
            if (event.type === "request.opened") {
              yield* Deferred.succeed(approvalOpened, undefined);
            }
            if (event.type === "turn.completed") {
              yield* Deferred.succeed(completed, undefined);
            }
          }),
        ),
        Effect.forkChild,
      );

      yield* adapter.startSession({
        threadId,
        provider: ProviderDriverKind.make("primeAgent"),
        providerInstanceId: ProviderInstanceId.make("primeAgent"),
        cwd: process.cwd(),
        runtimeMode: "approval-required",
      });
      yield* adapter.sendTurn({ threadId, input: "approval side effect", attachments: [] });
      yield* Deferred.await(approvalOpened);
      yield* adapter.respondToRequest(
        threadId,
        ApprovalRequestId.make("prime-approval-1"),
        "decline",
      );
      yield* Deferred.await(completed);

      const recorded = (yield* Effect.promise(() => NodeFSP.readFile(requestLogPath, "utf8")))
        .trim()
        .split("\n")
        .map((line) => decodeRequestLog(line));
      const response = recorded.find((entry) => entry.command.type === "extension_ui_response");
      assert.deepEqual(response?.command, {
        type: "extension_ui_response",
        id: "prime-approval-1",
        confirmed: false,
      });
      assert.isFalse(
        yield* Effect.promise(() =>
          NodeFSP.stat(sideEffectPath).then(
            () => true,
            () => false,
          ),
        ),
      );

      yield* adapter.stopSession(threadId);
      yield* Fiber.interrupt(eventFiber);
    }),
  );

  it.effect("cancels a pending Prime approval when the session stops", () =>
    Effect.gen(function* () {
      const config = yield* ServerConfig;
      const requestLogPath = NodePath.join(config.baseDir, "prime-stop-approval.ndjson");
      const binaryPath = yield* Effect.promise(() =>
        makeMockPrimeWrapper({ T3_PRIME_MOCK_REQUEST_LOG_PATH: requestLogPath }),
      );
      const adapter = yield* makeTestAdapter(binaryPath, {
        instanceId: ProviderInstanceId.make("primeAgent"),
      });
      const threadId = ThreadId.make("prime-stop-approval-thread");
      const approvalOpened = yield* Deferred.make<void>();
      const approvalResolved = yield* Deferred.make<ProviderRuntimeEvent>();
      const eventFiber = yield* adapter.streamEvents.pipe(
        Stream.runForEach((event) =>
          Effect.gen(function* () {
            if (event.type === "request.opened") {
              yield* Deferred.succeed(approvalOpened, undefined);
            }
            if (event.type === "request.resolved") {
              yield* Deferred.succeed(approvalResolved, event);
            }
          }),
        ),
        Effect.forkChild,
      );

      yield* adapter.startSession({
        threadId,
        provider: ProviderDriverKind.make("primeAgent"),
        providerInstanceId: ProviderInstanceId.make("primeAgent"),
        cwd: process.cwd(),
        runtimeMode: "approval-required",
      });
      yield* adapter.sendTurn({ threadId, input: "approval side effect", attachments: [] });
      yield* Deferred.await(approvalOpened);
      yield* adapter.stopSession(threadId);
      const resolved = yield* Deferred.await(approvalResolved).pipe(Effect.timeout("2 seconds"));

      assert.equal(resolved.type, "request.resolved");
      if (resolved.type === "request.resolved") {
        assert.equal(resolved.requestId, "prime-approval-1");
        assert.equal(resolved.payload.decision, "cancel");
        assert.deepEqual(resolved.payload.resolution, {
          type: "extension_ui_response",
          id: "prime-approval-1",
          cancelled: true,
        });
      }
      const recorded = (yield* Effect.promise(() => NodeFSP.readFile(requestLogPath, "utf8")))
        .trim()
        .split("\n")
        .map((line) => decodeRequestLog(line));
      assert.deepInclude(
        recorded.map((entry) => entry.command),
        { type: "extension_ui_response", id: "prime-approval-1", cancelled: true },
      );

      yield* Fiber.interrupt(eventFiber);
    }),
  );

  it.effect("cancels a pending Prime approval when the turn is interrupted", () =>
    Effect.gen(function* () {
      const config = yield* ServerConfig;
      const requestLogPath = NodePath.join(config.baseDir, "prime-interrupt-approval.ndjson");
      const binaryPath = yield* Effect.promise(() =>
        makeMockPrimeWrapper({ T3_PRIME_MOCK_REQUEST_LOG_PATH: requestLogPath }),
      );
      const adapter = yield* makeTestAdapter(binaryPath, {
        instanceId: ProviderInstanceId.make("primeAgent"),
      });
      const threadId = ThreadId.make("prime-interrupt-approval-thread");
      const approvalOpened = yield* Deferred.make<void>();
      const approvalResolved = yield* Deferred.make<ProviderRuntimeEvent>();
      const eventFiber = yield* adapter.streamEvents.pipe(
        Stream.runForEach((event) =>
          Effect.gen(function* () {
            if (event.type === "request.opened") {
              yield* Deferred.succeed(approvalOpened, undefined);
            }
            if (event.type === "request.resolved") {
              yield* Deferred.succeed(approvalResolved, event);
            }
          }),
        ),
        Effect.forkChild,
      );

      yield* adapter.startSession({
        threadId,
        provider: ProviderDriverKind.make("primeAgent"),
        providerInstanceId: ProviderInstanceId.make("primeAgent"),
        cwd: process.cwd(),
        runtimeMode: "approval-required",
      });
      const turn = yield* adapter.sendTurn({
        threadId,
        input: "approval side effect",
        attachments: [],
      });
      yield* Deferred.await(approvalOpened);
      yield* adapter.interruptTurn(threadId, turn.turnId);
      const resolved = yield* Deferred.await(approvalResolved).pipe(Effect.timeout("2 seconds"));

      assert.equal(resolved.type, "request.resolved");
      if (resolved.type === "request.resolved") {
        assert.equal(resolved.payload.decision, "cancel");
      }
      const recorded = (yield* Effect.promise(() => NodeFSP.readFile(requestLogPath, "utf8")))
        .trim()
        .split("\n")
        .map((line) => decodeRequestLog(line));
      assert.deepInclude(
        recorded.map((entry) => entry.command),
        { type: "extension_ui_response", id: "prime-approval-1", cancelled: true },
      );

      yield* adapter.stopSession(threadId);
      yield* Fiber.interrupt(eventFiber);
    }),
  );

  it.effect("resolves a pending Prime approval when the RPC process exits", () =>
    Effect.gen(function* () {
      const binaryPath = yield* Effect.promise(() => makeMockPrimeWrapper());
      const adapter = yield* makeTestAdapter(binaryPath, {
        instanceId: ProviderInstanceId.make("primeAgent"),
      });
      const threadId = ThreadId.make("prime-exit-approval-thread");
      const approvalResolved = yield* Deferred.make<ProviderRuntimeEvent>();
      const sessionExited = yield* Deferred.make<void>();
      const eventFiber = yield* adapter.streamEvents.pipe(
        Stream.runForEach((event) =>
          Effect.gen(function* () {
            if (event.type === "request.resolved") {
              yield* Deferred.succeed(approvalResolved, event);
            }
            if (event.type === "session.exited") {
              yield* Deferred.succeed(sessionExited, undefined);
            }
          }),
        ),
        Effect.forkChild,
      );

      yield* adapter.startSession({
        threadId,
        provider: ProviderDriverKind.make("primeAgent"),
        providerInstanceId: ProviderInstanceId.make("primeAgent"),
        cwd: process.cwd(),
        runtimeMode: "approval-required",
      });
      yield* adapter.sendTurn({ threadId, input: "approval then exit", attachments: [] });
      const resolved = yield* Deferred.await(approvalResolved).pipe(Effect.timeout("2 seconds"));

      assert.equal(resolved.type, "request.resolved");
      if (resolved.type === "request.resolved") {
        assert.equal(resolved.requestId, "prime-approval-exit");
        assert.equal(resolved.payload.decision, "cancel");
      }
      yield* Deferred.await(sessionExited);
      assert.isFalse(yield* adapter.hasSession(threadId));
      yield* Fiber.interrupt(eventFiber);
    }),
  );

  it.effect("maps a Prime select request to T3 user input and returns the selected value", () =>
    Effect.gen(function* () {
      const config = yield* ServerConfig;
      const requestLogPath = NodePath.join(config.baseDir, "prime-select-input.ndjson");
      const binaryPath = yield* Effect.promise(() =>
        makeMockPrimeWrapper({ T3_PRIME_MOCK_REQUEST_LOG_PATH: requestLogPath }),
      );
      const adapter = yield* makeTestAdapter(binaryPath, {
        instanceId: ProviderInstanceId.make("primeAgent"),
      });
      const threadId = ThreadId.make("prime-select-input-thread");
      const inputRequested = yield* Deferred.make<ProviderRuntimeEvent>();
      const completed = yield* Deferred.make<void>();
      const eventFiber = yield* adapter.streamEvents.pipe(
        Stream.runForEach((event) =>
          Effect.gen(function* () {
            if (event.type === "user-input.requested") {
              yield* Deferred.succeed(inputRequested, event);
            }
            if (event.type === "turn.completed") {
              yield* Deferred.succeed(completed, undefined);
            }
          }),
        ),
        Effect.forkChild,
      );

      yield* adapter.startSession({
        threadId,
        provider: ProviderDriverKind.make("primeAgent"),
        providerInstanceId: ProviderInstanceId.make("primeAgent"),
        cwd: process.cwd(),
        runtimeMode: "approval-required",
      });
      yield* adapter.sendTurn({ threadId, input: "select input", attachments: [] });
      const requested = yield* Deferred.await(inputRequested).pipe(Effect.timeout("2 seconds"));

      assert.equal(requested.type, "user-input.requested");
      if (requested.type !== "user-input.requested") {
        return;
      }
      assert.equal(requested.requestId, "prime-select-1");
      assert.deepEqual(requested.payload.questions, [
        {
          id: "prime-select-1",
          header: "Choose environment",
          question: "Choose environment",
          options: [
            { label: "dev", description: "dev" },
            { label: "prod", description: "prod" },
          ],
          multiSelect: false,
        },
      ]);
      yield* adapter.respondToUserInput(threadId, ApprovalRequestId.make("prime-select-1"), {
        "prime-select-1": "prod",
      });
      yield* Deferred.await(completed);

      const recorded = (yield* Effect.promise(() => NodeFSP.readFile(requestLogPath, "utf8")))
        .trim()
        .split("\n")
        .map((line) => decodeRequestLog(line));
      assert.deepInclude(
        recorded.map((entry) => entry.command),
        { type: "extension_ui_response", id: "prime-select-1", value: "prod" },
      );

      yield* adapter.stopSession(threadId);
      yield* Fiber.interrupt(eventFiber);
    }),
  );

  it.effect("maps Prime input and editor requests to text answers", () =>
    Effect.gen(function* () {
      const config = yield* ServerConfig;
      const cases = [
        {
          prompt: "text input",
          requestId: "prime-input-1",
          expectedQuestion: "Name release\n\nPlaceholder: v1.2.3",
          answer: "v2.0.0",
        },
        {
          prompt: "editor input",
          requestId: "prime-editor-1",
          expectedQuestion: "Edit release notes\n\nCurrent text:\nOld notes",
          answer: "New notes\nSecond line",
        },
      ] as const;

      yield* Effect.forEach(
        cases,
        (testCase, index) =>
          Effect.gen(function* () {
            const requestLogPath = NodePath.join(
              config.baseDir,
              `prime-text-input-${index}.ndjson`,
            );
            const binaryPath = yield* Effect.promise(() =>
              makeMockPrimeWrapper({ T3_PRIME_MOCK_REQUEST_LOG_PATH: requestLogPath }),
            );
            const adapter = yield* makeTestAdapter(binaryPath, {
              instanceId: ProviderInstanceId.make("primeAgent"),
            });
            const threadId = ThreadId.make(`prime-text-input-${index}`);
            const inputRequested = yield* Deferred.make<ProviderRuntimeEvent>();
            const completed = yield* Deferred.make<void>();
            const eventFiber = yield* adapter.streamEvents.pipe(
              Stream.runForEach((event) =>
                Effect.gen(function* () {
                  if (event.type === "user-input.requested") {
                    yield* Deferred.succeed(inputRequested, event);
                  }
                  if (event.type === "turn.completed") {
                    yield* Deferred.succeed(completed, undefined);
                  }
                }),
              ),
              Effect.forkChild,
            );

            yield* adapter.startSession({
              threadId,
              provider: ProviderDriverKind.make("primeAgent"),
              providerInstanceId: ProviderInstanceId.make("primeAgent"),
              cwd: process.cwd(),
              runtimeMode: "approval-required",
            });
            yield* adapter.sendTurn({
              threadId,
              input: testCase.prompt,
              attachments: [],
            });
            const requested = yield* Deferred.await(inputRequested).pipe(
              Effect.timeout("2 seconds"),
            );
            assert.equal(requested.type, "user-input.requested");
            if (requested.type !== "user-input.requested") {
              return;
            }
            assert.equal(String(requested.requestId), testCase.requestId);
            assert.equal(requested.payload.questions[0]?.question, testCase.expectedQuestion);
            assert.deepEqual(requested.payload.questions[0]?.options, []);

            yield* adapter.respondToUserInput(
              threadId,
              ApprovalRequestId.make(testCase.requestId),
              { [testCase.requestId]: testCase.answer },
            );
            yield* Deferred.await(completed);
            const recorded = (yield* Effect.promise(() => NodeFSP.readFile(requestLogPath, "utf8")))
              .trim()
              .split("\n")
              .map((line) => decodeRequestLog(line));
            assert.deepInclude(
              recorded.map((entry) => entry.command),
              {
                type: "extension_ui_response",
                id: testCase.requestId,
                value: testCase.answer,
              },
            );
            yield* adapter.stopSession(threadId);
            yield* Fiber.interrupt(eventFiber);
          }),
        { discard: true },
      );
    }),
  );

  it.effect("cancels pending Prime user input when the session stops", () =>
    Effect.gen(function* () {
      const config = yield* ServerConfig;
      const requestLogPath = NodePath.join(config.baseDir, "prime-stop-input.ndjson");
      const binaryPath = yield* Effect.promise(() =>
        makeMockPrimeWrapper({ T3_PRIME_MOCK_REQUEST_LOG_PATH: requestLogPath }),
      );
      const adapter = yield* makeTestAdapter(binaryPath, {
        instanceId: ProviderInstanceId.make("primeAgent"),
      });
      const threadId = ThreadId.make("prime-stop-input-thread");
      const inputRequested = yield* Deferred.make<void>();
      const inputResolved = yield* Deferred.make<ProviderRuntimeEvent>();
      const eventFiber = yield* adapter.streamEvents.pipe(
        Stream.runForEach((event) =>
          Effect.gen(function* () {
            if (event.type === "user-input.requested") {
              yield* Deferred.succeed(inputRequested, undefined);
            }
            if (event.type === "user-input.resolved") {
              yield* Deferred.succeed(inputResolved, event);
            }
          }),
        ),
        Effect.forkChild,
      );

      yield* adapter.startSession({
        threadId,
        provider: ProviderDriverKind.make("primeAgent"),
        providerInstanceId: ProviderInstanceId.make("primeAgent"),
        cwd: process.cwd(),
        runtimeMode: "approval-required",
      });
      yield* adapter.sendTurn({ threadId, input: "select input", attachments: [] });
      yield* Deferred.await(inputRequested);
      yield* adapter.stopSession(threadId);
      const resolved = yield* Deferred.await(inputResolved).pipe(Effect.timeout("2 seconds"));

      assert.equal(resolved.type, "user-input.resolved");
      if (resolved.type === "user-input.resolved") {
        assert.equal(resolved.requestId, "prime-select-1");
        assert.deepEqual(resolved.payload.answers, {});
      }
      const recorded = (yield* Effect.promise(() => NodeFSP.readFile(requestLogPath, "utf8")))
        .trim()
        .split("\n")
        .map((line) => decodeRequestLog(line));
      assert.deepInclude(
        recorded.map((entry) => entry.command),
        { type: "extension_ui_response", id: "prime-select-1", cancelled: true },
      );
      yield* Fiber.interrupt(eventFiber);
    }),
  );

  it.effect("resolves pending Prime requests when the turn ends", () =>
    Effect.gen(function* () {
      const binaryPath = yield* Effect.promise(() => makeMockPrimeWrapper());
      const adapter = yield* makeTestAdapter(binaryPath, {
        instanceId: ProviderInstanceId.make("primeAgent"),
      });
      const threadId = ThreadId.make("prime-end-approval-thread");
      const approvalResolved = yield* Deferred.make<ProviderRuntimeEvent>();
      const turnCompleted = yield* Deferred.make<void>();
      const eventFiber = yield* adapter.streamEvents.pipe(
        Stream.runForEach((event) =>
          Effect.gen(function* () {
            if (event.type === "request.resolved") {
              yield* Deferred.succeed(approvalResolved, event);
            }
            if (event.type === "turn.completed") {
              yield* Deferred.succeed(turnCompleted, undefined);
            }
          }),
        ),
        Effect.forkChild,
      );

      yield* adapter.startSession({
        threadId,
        provider: ProviderDriverKind.make("primeAgent"),
        providerInstanceId: ProviderInstanceId.make("primeAgent"),
        cwd: process.cwd(),
        runtimeMode: "approval-required",
      });
      yield* adapter.sendTurn({ threadId, input: "approval then end", attachments: [] });
      const resolved = yield* Deferred.await(approvalResolved).pipe(Effect.timeout("2 seconds"));
      yield* Deferred.await(turnCompleted);

      assert.equal(resolved.type, "request.resolved");
      if (resolved.type === "request.resolved") {
        assert.equal(resolved.requestId, "prime-approval-end");
        assert.equal(resolved.payload.decision, "cancel");
      }
      assert.isTrue(yield* adapter.hasSession(threadId));
      yield* adapter.stopSession(threadId);
      yield* Fiber.interrupt(eventFiber);
    }),
  );

  it.effect("rejects runtime modes and launch arguments that bypass supervised approvals", () =>
    Effect.gen(function* () {
      const binaryPath = yield* Effect.promise(() => makeMockPrimeWrapper());
      const instanceId = ProviderInstanceId.make("primeAgent");
      const unsafeLaunchArgs = [
        "--extension /tmp/rogue.ts",
        "-e /tmp/rogue.ts",
        "--no-extensions",
        "-ne",
        "--t3-approval-mode=full-access",
      ] as const;

      yield* Effect.forEach(
        unsafeLaunchArgs,
        (launchArgs, index) =>
          Effect.gen(function* () {
            const adapter = yield* makeTestAdapter(binaryPath, { instanceId, launchArgs });
            const error = yield* adapter
              .startSession({
                threadId: ThreadId.make(`prime-unsafe-launch-${index}`),
                provider: ProviderDriverKind.make("primeAgent"),
                providerInstanceId: instanceId,
                cwd: process.cwd(),
                runtimeMode: "approval-required",
              })
              .pipe(Effect.flip);
            assert.equal(error._tag, "ProviderAdapterValidationError");
            assert.include(error.message, "override the T3 approval extension");
          }),
        { discard: true },
      );

      yield* Effect.forEach(
        ["auto-accept-edits", "auto"] as const,
        (runtimeMode) =>
          Effect.gen(function* () {
            const adapter = yield* makeTestAdapter(binaryPath, { instanceId });
            const error = yield* adapter
              .startSession({
                threadId: ThreadId.make(`prime-unsupported-${runtimeMode}`),
                provider: ProviderDriverKind.make("primeAgent"),
                providerInstanceId: instanceId,
                cwd: process.cwd(),
                runtimeMode,
              })
              .pipe(Effect.flip);
            assert.equal(error._tag, "ProviderAdapterValidationError");
            assert.include(error.message, `does not support runtime mode '${runtimeMode}'`);
          }),
        { discard: true },
      );
    }),
  );

  it.effect("fails supervised sessions with invalid approval extension metadata", () =>
    Effect.gen(function* () {
      yield* Effect.forEach(
        ["missing", "wrong-path", "wrong-source", "malformed"] as const,
        (approvalHandshake) =>
          Effect.gen(function* () {
            const binaryPath = yield* Effect.promise(() =>
              makeMockPrimeWrapper({
                T3_PRIME_MOCK_APPROVAL_HANDSHAKE: approvalHandshake,
              }),
            );
            const adapter = yield* makeTestAdapter(binaryPath, {
              instanceId: ProviderInstanceId.make("primeAgent"),
            });
            const threadId = ThreadId.make(`prime-invalid-handshake-${approvalHandshake}`);

            const error = yield* adapter
              .startSession({
                threadId,
                provider: ProviderDriverKind.make("primeAgent"),
                providerInstanceId: ProviderInstanceId.make("primeAgent"),
                cwd: process.cwd(),
                runtimeMode: "approval-required",
              })
              .pipe(Effect.flip);

            assert.equal(error._tag, "ProviderAdapterRequestError");
            assert.include(error.message, "approval extension handshake");
            assert.isFalse(yield* adapter.hasSession(threadId));
          }),
        { discard: true },
      );
    }),
  );

  it.effect("starts a thread, streams one turn, and persists a resume cursor", () =>
    Effect.gen(function* () {
      const config = yield* ServerConfig;
      const binaryPath = yield* Effect.promise(() => makeMockPrimeWrapper());
      const adapter = yield* makeTestAdapter(binaryPath, {
        instanceId: ProviderInstanceId.make("primeAgent"),
      });
      const threadId = ThreadId.make("prime-thread");
      const events: ProviderRuntimeEvent[] = [];
      const completed = yield* Deferred.make<void>();
      const eventFiber = yield* adapter.streamEvents.pipe(
        Stream.runForEach((event) =>
          Effect.sync(() => events.push(event)).pipe(
            Effect.andThen(
              event.type === "turn.completed"
                ? Deferred.succeed(completed, undefined)
                : Effect.void,
            ),
          ),
        ),
        Effect.forkChild,
      );

      const session = yield* adapter.startSession({
        threadId,
        provider: ProviderDriverKind.make("primeAgent"),
        providerInstanceId: ProviderInstanceId.make("primeAgent"),
        cwd: process.cwd(),
        runtimeMode: "full-access",
      });

      assert.equal(session.provider, "primeAgent");
      assert.equal(adapter.capabilities.sessionModelSwitch, "in-session");
      assert.deepStrictEqual(session.resumeCursor, {
        schemaVersion: 1,
        sessionId: "prime-mock-session",
        sessionFile: NodePath.join(
          config.baseDir,
          "prime-agent",
          "sessions",
          String(threadId),
          "prime-mock-session.jsonl",
        ),
      });

      const turn = yield* adapter.sendTurn({ threadId, input: "hello", attachments: [] });
      yield* Deferred.await(completed);

      const delta = events.find(
        (event) => event.type === "content.delta" && event.payload.streamKind === "assistant_text",
      );
      assert.isDefined(delta);
      if (delta?.type === "content.delta") {
        assert.equal(delta.payload.delta, "hello from mock");
      }
      assert.include(
        events.map((event) => event.type),
        "turn.started",
      );
      assert.include(
        events.map((event) => event.type),
        "turn.completed",
      );
      assert.deepStrictEqual(turn.resumeCursor, session.resumeCursor);

      yield* adapter.stopSession(threadId);
      yield* Fiber.interrupt(eventFiber);
    }),
  );

  it.effect("maps thinking deltas and replaces cumulative tool partial results", () =>
    Effect.gen(function* () {
      const binaryPath = yield* Effect.promise(() => makeMockPrimeWrapper());
      const adapter = yield* makeTestAdapter(binaryPath);
      const threadId = ThreadId.make("prime-tools-thread");
      const events: ProviderRuntimeEvent[] = [];
      const completed = yield* Deferred.make<void>();
      yield* adapter.streamEvents.pipe(
        Stream.runForEach((event) =>
          Effect.sync(() => events.push(event)).pipe(
            Effect.andThen(
              event.type === "turn.completed"
                ? Deferred.succeed(completed, undefined)
                : Effect.void,
            ),
          ),
        ),
        Effect.forkChild,
      );

      yield* adapter.startSession({
        threadId,
        provider: ProviderDriverKind.make("primeAgent"),
        cwd: process.cwd(),
        runtimeMode: "full-access",
      });
      yield* adapter.sendTurn({ threadId, input: "emit tools", attachments: [] });
      yield* Deferred.await(completed);

      assert.isTrue(
        events.some(
          (event) =>
            event.type === "content.delta" && event.payload.streamKind === "reasoning_text",
        ),
      );
      assert.include(
        events.map((event) => event.type),
        "item.started",
      );
      assert.include(
        events.map((event) => event.type),
        "item.updated",
      );
      assert.include(
        events.map((event) => event.type),
        "item.completed",
      );

      const updates = events.filter(
        (event): event is Extract<ProviderRuntimeEvent, { type: "item.updated" }> =>
          event.type === "item.updated",
      );
      assert.equal(updates.length, 2);
      const firstResult = (updates[0]?.payload.data as { result?: unknown } | undefined)?.result;
      const secondResult = (updates[1]?.payload.data as { result?: unknown } | undefined)?.result;
      assert.deepStrictEqual(firstResult, { content: [{ type: "text", text: "2" }] });
      assert.deepStrictEqual(secondResult, { content: [{ type: "text", text: "2\n3" }] });

      yield* adapter.stopSession(threadId);
    }),
  );

  it.effect("steers a running turn instead of opening a new one on mid-turn sendTurn", () =>
    Effect.gen(function* () {
      const config = yield* ServerConfig;
      const requestLogPath = NodePath.join(config.baseDir, "prime-steer-requests.ndjson");
      const binaryPath = yield* Effect.promise(() =>
        makeMockPrimeWrapper({ T3_PRIME_MOCK_REQUEST_LOG_PATH: requestLogPath }),
      );
      const adapter = yield* makeTestAdapter(binaryPath);
      const threadId = ThreadId.make("prime-steer-thread");

      const runtimeEventsFiber = yield* adapter.streamEvents.pipe(
        Stream.takeUntil((event) => event.type === "turn.completed"),
        Stream.runCollect,
        Effect.forkChild,
      );

      yield* adapter.startSession({
        threadId,
        provider: ProviderDriverKind.make("primeAgent"),
        cwd: process.cwd(),
        runtimeMode: "full-access",
      });
      const turn = yield* adapter.sendTurn({ threadId, input: "slow turn", attachments: [] });

      // A mid-turn sendTurn is a steer: the message joins the live Prime turn.
      const steered = yield* adapter.sendTurn({ threadId, input: "steer now", attachments: [] });
      assert.equal(String(steered.turnId), String(turn.turnId));

      const runtimeEvents = Array.from(yield* Fiber.join(runtimeEventsFiber));
      const startedEvents = runtimeEvents.filter((event) => event.type === "turn.started");
      const completedEvents = runtimeEvents.filter((event) => event.type === "turn.completed");
      assert.equal(startedEvents.length, 1);
      assert.equal(String(startedEvents[0]?.turnId), String(turn.turnId));
      assert.equal(completedEvents.length, 1);
      assert.equal(String(completedEvents[0]?.turnId), String(turn.turnId));
      const deltas = runtimeEvents.flatMap((event) =>
        event.type === "content.delta" && event.payload.streamKind === "assistant_text"
          ? [event.payload.delta]
          : [],
      );
      assert.isTrue(deltas.some((text) => text.includes("too late") && text.includes("steer now")));

      const log = yield* waitForFileContent(requestLogPath, 80, '"steer"');
      const steerLine = log
        .split("\n")
        .map((line) => line.trim())
        .filter((line) => line.length > 0)
        .map((line) => decodeRequestLog(line))
        .find((recorded) => recorded.command.type === "steer");
      assert.isDefined(steerLine);
      if (steerLine !== undefined) {
        const steerCommand = steerLine.command as { type: string; message?: string };
        assert.equal(steerCommand.message, "steer now");
      }

      yield* adapter.stopSession(threadId);
    }),
  );

  it.effect("abort interrupts an in-flight turn and allows the next turn", () =>
    Effect.gen(function* () {
      const binaryPath = yield* Effect.promise(() =>
        makeMockPrimeWrapper({ T3_PRIME_MOCK_SUSPEND_AFTER_ABORT: "1" }),
      );
      const adapter = yield* makeTestAdapter(binaryPath);
      const threadId = ThreadId.make("prime-abort-thread");
      const aborted = yield* Deferred.make<void>();
      const interrupted = yield* Deferred.make<void>();
      const completed = yield* Deferred.make<void>();
      yield* adapter.streamEvents.pipe(
        Stream.runForEach((event) => {
          if (event.type === "turn.aborted") {
            return Deferred.succeed(aborted, undefined).pipe(Effect.ignore);
          }
          if (event.type === "turn.completed" && event.payload.state === "interrupted") {
            return Deferred.succeed(interrupted, undefined).pipe(Effect.ignore);
          }
          if (event.type === "turn.completed" && event.payload.state === "completed") {
            return Deferred.succeed(completed, undefined).pipe(Effect.ignore);
          }
          return Effect.void;
        }),
        Effect.forkChild,
      );

      yield* adapter.startSession({
        threadId,
        provider: ProviderDriverKind.make("primeAgent"),
        cwd: process.cwd(),
        runtimeMode: "full-access",
      });
      yield* adapter.sendTurn({ threadId, input: "slow turn", attachments: [] });
      yield* adapter.interruptTurn(threadId);
      yield* Deferred.await(aborted);
      yield* Deferred.await(interrupted);

      const completedRace = yield* Deferred.await(completed).pipe(
        Effect.timeoutOption("50 millis"),
      );
      assert.isTrue(Option.isNone(completedRace));

      yield* adapter.sendTurn({ threadId, input: "turn after abort", attachments: [] });
      yield* Deferred.await(completed);

      yield* adapter.stopSession(threadId);
    }),
  );

  it.effect("stopSession closes stdin so the mock exits on EOF", () =>
    Effect.gen(function* () {
      const fileSystem = yield* FileSystem.FileSystem;
      const now = yield* Clock.currentTimeMillis;
      const exitLogPath = NodePath.join(
        NodeOS.tmpdir(),
        `t3-prime-adapter-exit-${process.pid}-${now}.log`,
      );
      yield* fileSystem.writeFileString(exitLogPath, "");
      const binaryPath = yield* Effect.promise(() =>
        makeMockPrimeWrapper({ T3_PRIME_MOCK_EXIT_LOG_PATH: exitLogPath }),
      );
      const adapter = yield* makeTestAdapter(binaryPath);
      const threadId = ThreadId.make("prime-stop-thread");

      yield* adapter.startSession({
        threadId,
        provider: ProviderDriverKind.make("primeAgent"),
        cwd: process.cwd(),
        runtimeMode: "full-access",
      });
      yield* adapter.stopSession(threadId);

      const log = yield* waitForFileContent(exitLogPath, 80, "stdin-eof");
      assert.include(log, "stdin-eof");
    }),
  );

  it.effect("spawns RPC with cwd, session dir, daemon socket, and launch args", () =>
    Effect.gen(function* () {
      const config = yield* ServerConfig;
      const now = yield* Clock.currentTimeMillis;
      const requestLogPath = NodePath.join(
        NodeOS.tmpdir(),
        `t3-prime-adapter-args-${process.pid}-${now}.log`,
      );
      const binaryPath = yield* Effect.promise(() =>
        makeMockPrimeWrapper({ T3_PRIME_MOCK_REQUEST_LOG_PATH: requestLogPath }),
      );
      const adapter = yield* makeTestAdapter(binaryPath, {
        launchArgs: "--thinking high",
      });
      const threadId = ThreadId.make("prime-args-thread");

      yield* adapter.startSession({
        threadId,
        provider: ProviderDriverKind.make("primeAgent"),
        cwd: process.cwd(),
        runtimeMode: "full-access",
      });

      const log = yield* waitForFileContent(requestLogPath, 80, "get_state");
      const firstLine = log.split("\n").find((line) => line.trim().length > 0);
      assert.isDefined(firstLine);
      const recorded = decodeRequestLog(firstLine!);
      assert.includeMembers(
        [...recorded.args],
        [
          "--mode",
          "rpc",
          "--cwd",
          process.cwd(),
          "--session-dir",
          NodePath.join(config.baseDir, "prime-agent", "sessions", String(threadId)),
          "--daemon-socket",
          NodePath.join(config.baseDir, "prime-agent", "daemon.sock"),
          "--thinking",
          "high",
        ],
      );

      yield* adapter.stopSession(threadId);
    }),
  );

  it.effect("reports assistant errors as failed turns", () =>
    Effect.gen(function* () {
      const binaryPath = yield* Effect.promise(() => makeMockPrimeWrapper());
      const adapter = yield* makeTestAdapter(binaryPath);
      const threadId = ThreadId.make("prime-error-thread");
      const completed = yield* Deferred.make<ProviderRuntimeEvent>();
      yield* adapter.streamEvents.pipe(
        Stream.runForEach((event) =>
          event.type === "turn.completed" ? Deferred.succeed(completed, event) : Effect.void,
        ),
        Effect.forkChild,
      );

      yield* adapter.startSession({
        threadId,
        provider: ProviderDriverKind.make("primeAgent"),
        cwd: process.cwd(),
        runtimeMode: "full-access",
      });
      yield* adapter.sendTurn({ threadId, input: "fail prompt", attachments: [] });

      const event = yield* Deferred.await(completed);
      assert.equal(event.type, "turn.completed");
      if (event.type === "turn.completed") {
        assert.equal(event.payload.state, "failed");
        assert.equal(event.payload.stopReason, "error");
        assert.equal(event.payload.errorMessage, "Prime model rejected the request.");
      }
      yield* adapter.stopSession(threadId);
    }),
  );

  it.effect("resumes with --resume and continues the same Prime session", () =>
    Effect.gen(function* () {
      const now = yield* Clock.currentTimeMillis;
      const requestLogPath = NodePath.join(
        NodeOS.tmpdir(),
        `t3-prime-adapter-resume-${process.pid}-${now}.log`,
      );
      const binaryPath = yield* Effect.promise(() =>
        makeMockPrimeWrapper({ T3_PRIME_MOCK_REQUEST_LOG_PATH: requestLogPath }),
      );
      const adapter = yield* makeTestAdapter(binaryPath);
      const threadId = ThreadId.make("prime-resume-thread");
      const watchingResume = yield* Deferred.make<void>();
      const firstCompleted = yield* Deferred.make<void>();
      const recalled = yield* Deferred.make<string>();
      yield* adapter.streamEvents.pipe(
        Stream.runForEach((event) =>
          Effect.gen(function* () {
            const resumeWatching = yield* Deferred.isDone(watchingResume);
            if (!resumeWatching && event.type === "turn.completed") {
              yield* Deferred.succeed(firstCompleted, undefined).pipe(Effect.ignore);
              return;
            }
            if (
              resumeWatching &&
              event.type === "content.delta" &&
              event.payload.streamKind === "assistant_text"
            ) {
              yield* Deferred.succeed(recalled, event.payload.delta).pipe(Effect.ignore);
            }
          }),
        ),
        Effect.forkChild,
      );

      const session = yield* adapter.startSession({
        threadId,
        provider: ProviderDriverKind.make("primeAgent"),
        cwd: process.cwd(),
        runtimeMode: "full-access",
      });
      yield* adapter.sendTurn({ threadId, input: "remember mango", attachments: [] });
      yield* Deferred.await(firstCompleted);
      yield* adapter.stopSession(threadId);

      const resumeCursor = session.resumeCursor as {
        schemaVersion: number;
        sessionId: string;
        sessionFile: string;
      };
      const resumed = yield* adapter.startSession({
        threadId,
        provider: ProviderDriverKind.make("primeAgent"),
        cwd: process.cwd(),
        runtimeMode: "full-access",
        resumeCursor,
      });
      assert.deepStrictEqual(resumed.resumeCursor, resumeCursor);
      yield* Deferred.succeed(watchingResume, undefined);

      yield* adapter.sendTurn({ threadId, input: "recall last", attachments: [] });
      assert.equal(yield* Deferred.await(recalled), "remember mango");

      const log = yield* waitForFileContent(requestLogPath, 80, "--resume");
      const resumedGetState = log
        .split("\n")
        .filter((line) => line.trim().length > 0)
        .map((line) => decodeRequestLog(line))
        .find((entry) => entry.command.type === "get_state" && entry.args.includes("--resume"));
      assert.isDefined(resumedGetState);
      const resumeIndex = resumedGetState!.args.indexOf("--resume");
      assert.equal(resumedGetState!.args[resumeIndex + 1], resumeCursor.sessionFile);

      yield* adapter.stopSession(threadId);
    }),
  );

  it.effect("fails closed when the resume session file is outside the thread directory", () =>
    Effect.gen(function* () {
      const fileSystem = yield* FileSystem.FileSystem;
      const now = yield* Clock.currentTimeMillis;
      const outsideFile = NodePath.join(
        NodeOS.tmpdir(),
        `t3-prime-outside-${process.pid}-${now}.jsonl`,
      );
      yield* fileSystem.writeFileString(outsideFile, "");
      const binaryPath = yield* Effect.promise(() => makeMockPrimeWrapper());
      const adapter = yield* makeTestAdapter(binaryPath);
      const started = yield* adapter
        .startSession({
          threadId: ThreadId.make("prime-outside-thread"),
          provider: ProviderDriverKind.make("primeAgent"),
          cwd: process.cwd(),
          runtimeMode: "full-access",
          resumeCursor: {
            schemaVersion: 1,
            sessionId: "prime-mock-session",
            sessionFile: outsideFile,
          },
        })
        .pipe(Effect.result);

      assert.equal(started._tag, "Failure");
      if (started._tag === "Failure") {
        assert.include(started.failure.message, "outside");
      }
    }),
  );

  it.effect("fails closed when the resume session file is missing", () =>
    Effect.gen(function* () {
      const config = yield* ServerConfig;
      const threadId = ThreadId.make("prime-missing-file-thread");
      const sessionFile = NodePath.join(
        config.baseDir,
        "prime-agent",
        "sessions",
        String(threadId),
        "missing.jsonl",
      );
      const binaryPath = yield* Effect.promise(() => makeMockPrimeWrapper());
      const adapter = yield* makeTestAdapter(binaryPath);
      const started = yield* adapter
        .startSession({
          threadId,
          provider: ProviderDriverKind.make("primeAgent"),
          cwd: process.cwd(),
          runtimeMode: "full-access",
          resumeCursor: {
            schemaVersion: 1,
            sessionId: "prime-mock-session",
            sessionFile,
          },
        })
        .pipe(Effect.result);

      assert.equal(started._tag, "Failure");
      if (started._tag === "Failure") {
        assert.include(started.failure.message, "missing");
      }
    }),
  );

  it.effect("fails closed when the resumed session id does not match", () =>
    Effect.gen(function* () {
      const config = yield* ServerConfig;
      const fileSystem = yield* FileSystem.FileSystem;
      const threadId = ThreadId.make("prime-mismatch-thread");
      const sessionDir = NodePath.join(config.baseDir, "prime-agent", "sessions", String(threadId));
      const sessionFile = NodePath.join(sessionDir, "prime-mock-session.jsonl");
      yield* fileSystem.makeDirectory(sessionDir, { recursive: true });
      yield* fileSystem.writeFileString(sessionFile, "");
      const binaryPath = yield* Effect.promise(() => makeMockPrimeWrapper());
      const adapter = yield* makeTestAdapter(binaryPath);
      const started = yield* adapter
        .startSession({
          threadId,
          provider: ProviderDriverKind.make("primeAgent"),
          cwd: process.cwd(),
          runtimeMode: "full-access",
          resumeCursor: {
            schemaVersion: 1,
            sessionId: "not-the-mock-session",
            sessionFile,
          },
        })
        .pipe(Effect.result);

      assert.equal(started._tag, "Failure");
      if (started._tag === "Failure") {
        assert.include(started.failure.message, "did not match");
      }
    }),
  );

  it.effect("ignores a malformed resume cursor and starts a fresh session", () =>
    Effect.gen(function* () {
      const now = yield* Clock.currentTimeMillis;
      const requestLogPath = NodePath.join(
        NodeOS.tmpdir(),
        `t3-prime-adapter-badcursor-${process.pid}-${now}.log`,
      );
      const binaryPath = yield* Effect.promise(() =>
        makeMockPrimeWrapper({ T3_PRIME_MOCK_REQUEST_LOG_PATH: requestLogPath }),
      );
      const adapter = yield* makeTestAdapter(binaryPath);
      const threadId = ThreadId.make("prime-badcursor-thread");
      const session = yield* adapter.startSession({
        threadId,
        provider: ProviderDriverKind.make("primeAgent"),
        cwd: process.cwd(),
        runtimeMode: "full-access",
        resumeCursor: { schemaVersion: 99, sessionId: "stale", sessionFile: "/tmp/stale.jsonl" },
      });

      assert.equal(
        (session.resumeCursor as { sessionId?: string } | undefined)?.sessionId,
        "prime-mock-session",
      );
      const log = yield* waitForFileContent(requestLogPath, 80, "get_state");
      const recorded = decodeRequestLog(log.split("\n").find((line) => line.trim().length > 0)!);
      assert.notInclude(recorded.args, "--resume");

      yield* adapter.stopSession(threadId);
    }),
  );

  it.effect("switches model and thinking level in session", () =>
    Effect.gen(function* () {
      const now = yield* Clock.currentTimeMillis;
      const requestLogPath = NodePath.join(
        NodeOS.tmpdir(),
        `t3-prime-adapter-model-${process.pid}-${now}.log`,
      );
      const binaryPath = yield* Effect.promise(() =>
        makeMockPrimeWrapper({ T3_PRIME_MOCK_REQUEST_LOG_PATH: requestLogPath }),
      );
      const adapter = yield* makeTestAdapter(binaryPath, {
        instanceId: ProviderInstanceId.make("primeAgent"),
      });
      const threadId = ThreadId.make("prime-model-thread");
      const completed = yield* Deferred.make<void>();
      yield* adapter.streamEvents.pipe(
        Stream.runForEach((event) =>
          event.type === "turn.completed" ? Deferred.succeed(completed, undefined) : Effect.void,
        ),
        Effect.forkChild,
      );

      const session = yield* adapter.startSession({
        threadId,
        provider: ProviderDriverKind.make("primeAgent"),
        cwd: process.cwd(),
        runtimeMode: "full-access",
      });
      assert.equal(session.model, "openai-codex/gpt-5.6-sol");

      yield* adapter.sendTurn({
        threadId,
        input: "hello",
        attachments: [],
        modelSelection: createModelSelection(
          ProviderInstanceId.make("primeAgent"),
          "anthropic/claude-sonnet-4",
          [{ id: "thinkingLevel", value: "high" }],
        ),
      });
      yield* Deferred.await(completed);

      const log = yield* waitForFileContent(requestLogPath, 80, "set_model");
      const commands = log
        .split("\n")
        .filter((line) => line.trim().length > 0)
        .map((line) => decodeRequestLog(line).command);
      assert.isTrue(
        commands.some(
          (command) =>
            command.type === "set_model" &&
            command.provider === "anthropic" &&
            command.modelId === "claude-sonnet-4",
        ),
      );
      assert.isTrue(
        commands.some(
          (command) => command.type === "set_thinking_level" && command.level === "high",
        ),
      );

      yield* adapter.stopSession(threadId);
    }),
  );

  it.effect("maps Prime RLM child updates to task activities", () =>
    Effect.gen(function* () {
      const config = yield* ServerConfig;
      const requestLogPath = NodePath.join(config.baseDir, "prime-subagent-requests.ndjson");
      const binaryPath = yield* Effect.promise(() =>
        makeMockPrimeWrapper({ T3_PRIME_MOCK_REQUEST_LOG_PATH: requestLogPath }),
      );
      const adapter = yield* makeTestAdapter(binaryPath);
      const threadId = ThreadId.make("prime-subagents-thread");
      const startedEvents: Array<ProviderRuntimeEvent & { type: "task.started" }> = [];
      const progressEvents: Array<ProviderRuntimeEvent & { type: "task.progress" }> = [];
      const completedEvents: Array<ProviderRuntimeEvent & { type: "task.completed" }> = [];
      const fleetSettled = yield* Deferred.make<void>();
      const eventFiber = yield* adapter.streamEvents.pipe(
        Stream.runForEach((event) =>
          Effect.gen(function* () {
            if (event.type === "task.started") {
              startedEvents.push(event);
            }
            if (event.type === "task.progress") {
              progressEvents.push(event);
            }
            if (event.type === "task.completed") {
              completedEvents.push(event);
              // The mock emits this terminal row after the parent turn ends;
              // seeing it means every earlier row was already delivered.
              if (event.payload.taskId === "prime-sub-2") {
                yield* Deferred.succeed(fleetSettled, undefined);
              }
            }
          }),
        ),
        Effect.forkChild,
      );

      yield* adapter.startSession({
        threadId,
        provider: ProviderDriverKind.make("primeAgent"),
        providerInstanceId: ProviderInstanceId.make("primeAgent"),
        cwd: process.cwd(),
        runtimeMode: "full-access",
      });
      const turn = yield* adapter.sendTurn({
        threadId,
        input: "spawn subagents",
        attachments: [],
      });
      yield* Deferred.await(fleetSettled);

      const started = startedEvents.find((event) => event.payload.taskId === "prime-sub-1");
      assert.isDefined(started);
      assert.equal(started?.turnId, turn.turnId);
      assert.equal(started?.payload.taskType, "subagent");
      assert.equal(started?.payload.title, "audit-renderer");
      assert.equal(started?.payload.description, "Audit the pie renderer code");
      assert.equal(started?.payload.model, "openrouter/ox-alpha");

      const childOneProgress = progressEvents.filter(
        (event) => event.payload.taskId === "prime-sub-1",
      );
      // Prime emits an RLM snapshot for every text delta. The adapter only
      // forwards meaningful roster changes.
      assert.lengthOf(childOneProgress, 1);
      const progress = childOneProgress[0];
      assert.equal(progress?.payload.summary, "Using ipython");
      assert.equal(progress?.payload.status, "running");
      assert.equal(progress?.payload.lastToolName, "ipython");
      // Prime's tokenCount is current context size, not cumulative usage, so
      // the adapter must not present it as total token usage.
      assert.isUndefined(progress?.payload.typedUsage);
      // Identity rides on every row so folds survive activity retention.
      assert.equal(progress?.payload.title, "audit-renderer");

      const completedOne = completedEvents.find((event) => event.payload.taskId === "prime-sub-1");
      assert.isDefined(completedOne);
      assert.equal(completedOne?.payload.status, "completed");
      assert.equal(completedOne?.payload.summary, "Found one dead helper in normalize.js");
      assert.isUndefined(completedOne?.payload.typedUsage);

      const failedTwo = completedEvents.find((event) => event.payload.taskId === "prime-sub-2");
      assert.isDefined(failedTwo);
      assert.equal(failedTwo?.turnId, turn.turnId);
      assert.equal(failedTwo?.payload.status, "failed");
      // The failed child's error rides as the terminal summary (the contract's
      // task.completed payload has no error field; clients read it from there).
      assert.equal(failedTwo?.payload.summary, "Test suite could not start");

      yield* adapter.stopSession(threadId);
      yield* Fiber.interrupt(eventFiber);
    }),
  );
});
