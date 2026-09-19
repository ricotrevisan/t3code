import {
  ProviderAdapterHostProcessError,
  type ProviderAdapterProcessV1,
} from "@t3tools/provider-adapter";
import { assert, it } from "@effect/vitest";
import * as Cause from "effect/Cause";
import * as Deferred from "effect/Deferred";
import * as Effect from "effect/Effect";
import * as Fiber from "effect/Fiber";
import * as Queue from "effect/Queue";
import * as Ref from "effect/Ref";
import * as Stream from "effect/Stream";
import * as AcpError from "effect-acp/errors";

import * as AcpHostConnection from "./AcpHostConnection.ts";

const encoder = new TextEncoder();
const decoder = new TextDecoder();

interface WireMessage {
  readonly id?: string | number;
  readonly method?: string;
  readonly params?: unknown;
  readonly result?: unknown;
}

const decodeMessage = (chunk: Uint8Array): WireMessage => {
  const value: unknown = JSON.parse(decoder.decode(chunk));
  if (typeof value !== "object" || value === null) {
    throw new Error("Expected an ACP object message");
  }
  return value as WireMessage;
};

const encodeMessage = (message: WireMessage): Uint8Array =>
  encoder.encode(`${JSON.stringify({ jsonrpc: "2.0", ...message })}\n`);

const makeFakeProcess = Effect.fn("makeFakeProcess")(function* (
  writeError?: ProviderAdapterHostProcessError,
) {
  const stdout = yield* Queue.unbounded<Uint8Array, Cause.Done<void>>();
  const stderr = yield* Queue.bounded<Uint8Array, Cause.Done<void>>(1);
  const writes = yield* Queue.unbounded<Uint8Array>();
  const exitCode = yield* Deferred.make<number, ProviderAdapterHostProcessError>();
  const writeAttempted = yield* Deferred.make<void>();
  const closeCount = yield* Ref.make(0);
  const expectExitCount = yield* Ref.make(0);

  const process = {
    pid: 4242,
    attachSession: () => Effect.void,
    detachSession: () => Effect.void,
    expectExit: Ref.update(expectExitCount, (count) => count + 1),
    stdout: Stream.fromQueue(stdout),
    stderr: Stream.fromQueue(stderr),
    exitCode: Deferred.await(exitCode),
    write: (chunk) =>
      Deferred.succeed(writeAttempted, undefined).pipe(
        Effect.andThen(
          writeError ? Effect.fail(writeError) : Queue.offer(writes, chunk).pipe(Effect.asVoid),
        ),
      ),
    close: Ref.update(closeCount, (count) => count + 1),
  } satisfies ProviderAdapterProcessV1;

  return {
    process,
    stdout,
    stderr,
    writes,
    exitCode,
    writeAttempted,
    closeCount,
    expectExitCount,
  };
});

const failMessage = "Expected the ACP request to fail";

it.effect("correlates initialize while serving an interleaved permission request", () =>
  Effect.scoped(
    Effect.gen(function* () {
      const fake = yield* makeFakeProcess();
      const connection = yield* AcpHostConnection.make(fake.process);
      const permissionRequests = yield* Ref.make<Array<unknown>>([]);

      yield* connection.client.handleRequestPermission((request) =>
        Ref.update(permissionRequests, (requests) => [...requests, request]).pipe(
          Effect.as({
            outcome: {
              outcome: "selected" as const,
              optionId: "allow",
            },
          }),
        ),
      );

      const initialize = yield* connection.client.agent
        .initialize({
          protocolVersion: 1,
          clientInfo: { name: "t3-test", version: "0.0.0" },
        })
        .pipe(Effect.forkScoped);

      const initializeRequest = decodeMessage(yield* Queue.take(fake.writes));
      assert.equal(initializeRequest.method, "initialize");
      assert.isDefined(initializeRequest.id);

      yield* Queue.offer(
        fake.stdout,
        encodeMessage({
          id: 91,
          method: "session/request_permission",
          params: {
            sessionId: "session-1",
            options: [{ optionId: "allow", name: "Allow", kind: "allow_once" }],
            toolCall: { toolCallId: "tool-1", title: "Read project files" },
          },
        }),
      );

      const permissionResponse = decodeMessage(yield* Queue.take(fake.writes));
      assert.equal(permissionResponse.id, 91);
      assert.deepEqual(permissionResponse.result, {
        outcome: { outcome: "selected", optionId: "allow" },
      });

      yield* Queue.offer(
        fake.stdout,
        encodeMessage({
          id: initializeRequest.id,
          result: {
            protocolVersion: 1,
            agentInfo: { name: "fake-agent", version: "1.0.0" },
          },
        }),
      );

      const initialized = yield* Fiber.join(initialize);
      assert.equal(initialized.protocolVersion, 1);
      assert.equal(initialized.agentInfo?.name, "fake-agent");
      assert.equal((yield* Ref.get(permissionRequests)).length, 1);
      assert.strictEqual(connection.process, fake.process);
    }),
  ),
);

it.effect("maps a host write failure and fails the pending call", () =>
  Effect.scoped(
    Effect.gen(function* () {
      const writeError = new ProviderAdapterHostProcessError({
        operation: "write",
        detail: "stdin closed",
      });
      const fake = yield* makeFakeProcess(writeError);
      const connection = yield* AcpHostConnection.make(fake.process);
      const pending = yield* connection.client.raw.request("x/pending", {}).pipe(Effect.forkScoped);

      yield* Deferred.await(fake.writeAttempted);
      const error = yield* Fiber.join(pending).pipe(
        Effect.match({
          onFailure: (error) => error,
          onSuccess: () => assert.fail(failMessage),
        }),
      );

      assert.instanceOf(error, AcpError.AcpTransportError);
      assert.equal(error.operation, "read-input-stream");
    }),
  ),
);

it.effect("maps EOF plus process exit and fails pending calls", () =>
  Effect.scoped(
    Effect.gen(function* () {
      const fake = yield* makeFakeProcess();
      const connection = yield* AcpHostConnection.make(fake.process);
      const pending = yield* connection.client.raw.request("x/pending", {}).pipe(Effect.forkScoped);

      yield* Queue.take(fake.writes);
      yield* Deferred.succeed(fake.exitCode, 17);
      yield* Queue.end(fake.stdout);

      const error = yield* Fiber.join(pending).pipe(
        Effect.match({
          onFailure: (error) => error,
          onSuccess: () => assert.fail(failMessage),
        }),
      );

      assert.instanceOf(error, AcpError.AcpProcessExitedError);
      assert.deepInclude(error, { code: 17, pid: 4242 });
    }),
  ),
);

it.effect("maps exit status lookup failures to transport errors", () =>
  Effect.scoped(
    Effect.gen(function* () {
      const fake = yield* makeFakeProcess();
      const connection = yield* AcpHostConnection.make(fake.process);
      const pending = yield* connection.client.raw.request("x/pending", {}).pipe(Effect.forkScoped);
      const exitError = new ProviderAdapterHostProcessError({
        operation: "exitCode",
        detail: "status unavailable",
      });

      yield* Queue.take(fake.writes);
      yield* Deferred.fail(fake.exitCode, exitError);
      yield* Queue.end(fake.stdout);

      const error = yield* Fiber.join(pending).pipe(
        Effect.match({
          onFailure: (error) => error,
          onSuccess: () => assert.fail(failMessage),
        }),
      );

      assert.instanceOf(error, AcpError.AcpTransportError);
      assert.deepInclude(error, {
        operation: "read-process-exit-status",
        pid: 4242,
        cause: exitError,
      });
    }),
  ),
);

it.effect("passes initial typed handlers before reading queued startup notifications", () =>
  Effect.scoped(
    Effect.gen(function* () {
      const fake = yield* makeFakeProcess();
      const handled = yield* Deferred.make<unknown>();

      yield* Queue.offer(
        fake.stdout,
        encodeMessage({
          method: "session/update",
          params: {
            sessionId: "session-startup",
            update: {
              sessionUpdate: "plan",
              entries: [
                {
                  content: "Queued before connection startup",
                  priority: "high",
                  status: "in_progress",
                },
              ],
            },
          },
        }),
      );

      const connection = yield* AcpHostConnection.make(fake.process, {
        onSessionUpdate: (notification) =>
          Deferred.succeed(handled, notification).pipe(Effect.asVoid),
      });

      assert.deepEqual(yield* Deferred.await(handled), {
        sessionId: "session-startup",
        update: {
          sessionUpdate: "plan",
          entries: [
            {
              content: "Queued before connection startup",
              priority: "high",
              status: "in_progress",
            },
          ],
        },
      });
      assert.equal((yield* Stream.runCollect(connection.client.raw.notifications)).length, 0);
    }),
  ),
);

it.effect("dispatches sustained large typed notifications without retaining raw duplicates", () =>
  Effect.scoped(
    Effect.gen(function* () {
      const notificationCount = 128;
      const largeContent = "x".repeat(32 * 1024);
      const fake = yield* makeFakeProcess();
      const connection = yield* AcpHostConnection.make(fake.process);
      const handledCount = yield* Ref.make(0);
      const allHandled = yield* Deferred.make<void>();

      yield* connection.client.handleSessionUpdate(() =>
        Ref.updateAndGet(handledCount, (count) => count + 1).pipe(
          Effect.flatMap((count) =>
            count === notificationCount
              ? Deferred.succeed(allHandled, undefined).pipe(Effect.asVoid)
              : Effect.void,
          ),
        ),
      );

      yield* Effect.forEach(
        Array.from({ length: notificationCount }, (_, index) => index),
        (index) =>
          Queue.offer(
            fake.stdout,
            encodeMessage({
              method: "session/update",
              params: {
                sessionId: "session-1",
                update: {
                  sessionUpdate: "plan",
                  entries: [
                    {
                      content: `${index}:${largeContent}`,
                      priority: "high",
                      status: "in_progress",
                    },
                  ],
                },
              },
            }),
          ),
        { discard: true },
      );

      yield* Deferred.await(allHandled);
      assert.equal(yield* Ref.get(handledCount), notificationCount);
      assert.equal((yield* Stream.runCollect(connection.client.raw.notifications)).length, 0);
    }),
  ),
);

it.effect("captures raw notifications only when explicitly enabled", () =>
  Effect.scoped(
    Effect.gen(function* () {
      const fake = yield* makeFakeProcess();
      const connection = yield* AcpHostConnection.make(fake.process, {
        captureRawNotifications: true,
      });
      const captured = yield* connection.client.raw.notifications.pipe(
        Stream.take(1),
        Stream.runCollect,
        Effect.forkScoped,
      );

      yield* Queue.offer(
        fake.stdout,
        encodeMessage({
          method: "x/captured",
          params: { payload: "raw" },
        }),
      );

      const notifications = yield* Fiber.join(captured);
      assert.equal(notifications.length, 1);
      assert.deepEqual(notifications[0], {
        _tag: "ExtNotification",
        method: "x/captured",
        params: { payload: "raw" },
      });
    }),
  ),
);

it.effect("drains stderr into a bounded tail without taking process ownership", () =>
  Effect.gen(function* () {
    const fake = yield* makeFakeProcess();
    const result = yield* Effect.scoped(
      Effect.gen(function* () {
        const connection = yield* AcpHostConnection.make(fake.process, { stderrTailBytes: 0 });
        yield* Queue.offer(fake.stderr, encoder.encode("first"));
        yield* Queue.offer(fake.stderr, encoder.encode("second"));
        yield* Queue.offer(fake.stderr, encoder.encode("third"));
        yield* Queue.offer(fake.stderr, encoder.encode("fourth"));
        return yield* connection.stderrTail;
      }),
    );

    assert.equal(result, "");
    assert.equal(yield* Ref.get(fake.expectExitCount), 0);
    assert.equal(yield* Ref.get(fake.closeCount), 0);
  }),
);
