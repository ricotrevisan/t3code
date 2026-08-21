import { resolveSpawnCommand } from "@t3tools/shared/shell";
import * as Cause from "effect/Cause";
import * as Deferred from "effect/Deferred";
import * as Duration from "effect/Duration";
import * as Effect from "effect/Effect";
import * as Option from "effect/Option";
import * as PubSub from "effect/PubSub";
import * as Queue from "effect/Queue";
import * as Ref from "effect/Ref";
import * as Schema from "effect/Schema";
import type * as Scope from "effect/Scope";
import * as Stream from "effect/Stream";
import * as ChildProcess from "effect/unstable/process/ChildProcess";
import * as ChildProcessSpawner from "effect/unstable/process/ChildProcessSpawner";

const PrimeRpcResponse = Schema.Struct({
  id: Schema.String,
  type: Schema.Literal("response"),
  command: Schema.String,
  success: Schema.Boolean,
  data: Schema.optional(Schema.Unknown),
  error: Schema.optional(Schema.String),
});

export type PrimeRpcResponse = typeof PrimeRpcResponse.Type;

export type PrimeExtensionUiResponse =
  | { readonly id: string; readonly value: string }
  | { readonly id: string; readonly confirmed: boolean }
  | { readonly id: string; readonly cancelled: true };

const decodeJson = Schema.decodeUnknownEffect(Schema.fromJsonString(Schema.Unknown));
const decodeResponse = Schema.decodeUnknownEffect(PrimeRpcResponse);
const encodeJson = Schema.encodeUnknownEffect(Schema.fromJsonString(Schema.Unknown));

export class PrimeRpcError extends Schema.TaggedErrorClass<PrimeRpcError>()("PrimeRpcError", {
  operation: Schema.String,
  detail: Schema.String,
  cause: Schema.optional(Schema.Defect()),
}) {
  override get message(): string {
    return `Prime RPC ${this.operation} failed: ${this.detail}`;
  }
}

export interface PrimeRpcClient {
  readonly pid: number;
  readonly request: (
    command: Readonly<Record<string, unknown>> & { readonly type: string },
    options?: { readonly timeoutMs?: number | null },
  ) => Effect.Effect<PrimeRpcResponse, PrimeRpcError>;
  readonly respondToExtensionUi: (
    response: PrimeExtensionUiResponse,
  ) => Effect.Effect<void, PrimeRpcError>;
  readonly events: Stream.Stream<unknown>;
  readonly close: Effect.Effect<void>;
}

export interface PrimeRpcSpawnOptions {
  readonly command: string;
  readonly args?: ReadonlyArray<string>;
  readonly cwd?: string;
  readonly environment?: NodeJS.ProcessEnv;
}

const encoder = new TextEncoder();

const failPending = (
  pending: Map<string, Deferred.Deferred<PrimeRpcResponse, PrimeRpcError>>,
  error: PrimeRpcError,
) =>
  Effect.gen(function* () {
    const waiters = [...pending.values()];
    pending.clear();
    yield* Effect.forEach(waiters, (waiter) => Deferred.fail(waiter, error), { discard: true });
  });

export const spawnPrimeRpcClient = Effect.fn("PrimeRpcClient.spawn")(function* (
  options: PrimeRpcSpawnOptions,
): Effect.fn.Return<
  PrimeRpcClient,
  PrimeRpcError,
  ChildProcessSpawner.ChildProcessSpawner | Scope.Scope
> {
  const spawner = yield* ChildProcessSpawner.ChildProcessSpawner;
  const args = options.args ?? [];
  const spawnCommand = yield* resolveSpawnCommand(
    options.command,
    args,
    options.environment ? { env: options.environment, extendEnv: true } : {},
  );
  const outgoing = yield* Queue.unbounded<Uint8Array, Cause.Done<void>>();
  const incoming = yield* PubSub.unbounded<unknown>({ replay: 64 });
  const pending = new Map<string, Deferred.Deferred<PrimeRpcResponse, PrimeRpcError>>();
  const closed = yield* Ref.make(false);
  let requestSequence = 0;
  let stderr = "";
  const decoder = new TextDecoder();

  const child = yield* spawner
    .spawn(
      ChildProcess.make(spawnCommand.command, spawnCommand.args, {
        cwd: options.cwd,
        ...(options.environment
          ? { env: options.environment, extendEnv: true }
          : { extendEnv: true }),
        shell: spawnCommand.shell,
        stdin: {
          stream: Stream.fromQueue(outgoing),
          endOnDone: true,
        },
        stdout: "pipe",
        stderr: "pipe",
        killSignal: "SIGTERM",
        forceKillAfter: Duration.seconds(2),
      }),
    )
    .pipe(
      Effect.mapError(
        (cause) =>
          new PrimeRpcError({
            operation: "spawn",
            detail: "Failed to start the process.",
            cause,
          }),
      ),
    );

  const dispatchLine = (line: string) =>
    Effect.gen(function* () {
      if (line.length === 0) {
        return;
      }
      const raw = yield* decodeJson(line).pipe(
        Effect.tapError((cause) =>
          Effect.logDebug("Prime RPC skipped a malformed JSON line.", { cause }),
        ),
        Effect.option,
      );
      if (Option.isNone(raw)) {
        return;
      }
      if (!isRecord(raw.value) || raw.value.type !== "response") {
        // PubSub is unbounded, so publishing preserves stdout order without
        // waiting for subscribers to process the event.
        yield* PubSub.publish(incoming, raw.value);
        return;
      }
      const response = yield* decodeResponse(raw.value).pipe(
        Effect.tapError((cause) =>
          Effect.logDebug("Prime RPC skipped a malformed response.", { cause }),
        ),
        Effect.option,
      );
      if (Option.isNone(response)) {
        return;
      }
      const waiter = pending.get(response.value.id);
      if (waiter !== undefined) {
        yield* Deferred.succeed(waiter, response.value);
      }
    });

  // Split stdout on `\n` only. Node readline also splits on U+2028/U+2029.
  yield* Effect.gen(function* () {
    let buffer = "";
    yield* Stream.runForEach(child.stdout, (chunk) =>
      Effect.gen(function* () {
        buffer += decoder.decode(chunk, { stream: true });
        while (true) {
          const newline = buffer.indexOf("\n");
          if (newline < 0) {
            break;
          }
          const line = buffer.slice(0, newline).replace(/\r$/, "");
          buffer = buffer.slice(newline + 1);
          yield* dispatchLine(line);
        }
      }),
    );
    buffer += decoder.decode();
    if (buffer.length > 0) {
      yield* dispatchLine(buffer.replace(/\r$/, ""));
    }
  }).pipe(
    Effect.catch((cause) => Effect.logError("Prime RPC stdout failed.", { cause })),
    Effect.forkScoped,
  );

  yield* child.stderr.pipe(
    Stream.decodeText(),
    Stream.runForEach((chunk) =>
      Effect.sync(() => {
        stderr = `${stderr}${chunk}`.slice(-4_000);
      }),
    ),
    Effect.catch((cause) => Effect.logDebug("Prime RPC stderr closed.", { cause })),
    Effect.forkScoped,
  );

  yield* child.exitCode.pipe(
    Effect.flatMap((exitCode) =>
      Effect.gen(function* () {
        const failure = new PrimeRpcError({
          operation: "process",
          detail: `Process exited with code ${Number(exitCode)}.${stderr.trim() ? ` ${stderr.trim()}` : ""}`,
        });
        yield* failPending(pending, failure);
        yield* PubSub.shutdown(incoming);
      }),
    ),
    Effect.catch((cause) => Effect.logDebug("Prime RPC exit monitor failed.", { cause })),
    Effect.forkScoped,
  );

  const close = Effect.gen(function* () {
    if (yield* Ref.getAndSet(closed, true)) {
      return;
    }
    yield* failPending(
      pending,
      new PrimeRpcError({
        operation: "process",
        detail: "RPC process closed.",
      }),
    );
    yield* Queue.end(outgoing);
    const exited = yield* child.exitCode.pipe(
      Effect.timeoutOption(Duration.seconds(2)),
      Effect.catch(() => Effect.succeedNone),
    );
    if (Option.isNone(exited)) {
      yield* child.kill().pipe(Effect.ignore);
    }
    yield* PubSub.shutdown(incoming);
  });

  yield* Effect.addFinalizer(() => close);

  // `prompt` success means accepted, not finished. Wait for `agent_end`.
  const request: PrimeRpcClient["request"] = (command, requestOptions) =>
    Effect.gen(function* () {
      if (yield* Ref.get(closed)) {
        return yield* new PrimeRpcError({
          operation: command.type,
          detail: "RPC process is closed.",
        });
      }
      const id = `t3-${++requestSequence}`;
      const response = yield* Deferred.make<PrimeRpcResponse, PrimeRpcError>();
      const payload = yield* encodeJson({ ...command, id }).pipe(
        Effect.mapError(
          (cause) =>
            new PrimeRpcError({
              operation: command.type,
              detail: "Command is not JSON-serializable.",
              cause,
            }),
        ),
      );
      pending.set(id, response);
      yield* Queue.offer(outgoing, encoder.encode(`${payload}\n`));
      const awaited = Deferred.await(response);
      const result = yield* (
        requestOptions?.timeoutMs === null
          ? awaited
          : awaited.pipe(
              Effect.timeoutOption(requestOptions?.timeoutMs ?? 30_000),
              Effect.flatMap(
                Option.match({
                  onNone: () =>
                    Effect.fail(
                      new PrimeRpcError({ operation: command.type, detail: "Request timed out." }),
                    ),
                  onSome: Effect.succeed,
                }),
              ),
            )
      ).pipe(Effect.ensuring(Effect.sync(() => pending.delete(id))));
      if (!result.success) {
        return yield* new PrimeRpcError({
          operation: command.type,
          detail: result.error ?? "Request failed.",
        });
      }
      return result;
    });

  const respondToExtensionUi: PrimeRpcClient["respondToExtensionUi"] = (response) =>
    Effect.gen(function* () {
      if (yield* Ref.get(closed)) {
        return yield* new PrimeRpcError({
          operation: "extension_ui_response",
          detail: "RPC process is closed.",
        });
      }
      const payload = yield* encodeJson({ type: "extension_ui_response", ...response }).pipe(
        Effect.mapError(
          (cause) =>
            new PrimeRpcError({
              operation: "extension_ui_response",
              detail: "Response is not JSON-serializable.",
              cause,
            }),
        ),
      );
      yield* Queue.offer(outgoing, encoder.encode(`${payload}\n`));
    });

  return {
    pid: Number(child.pid),
    request,
    respondToExtensionUi,
    events: Stream.fromPubSub(incoming),
    close,
  };
});

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}
