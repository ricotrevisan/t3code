import type {
  ProviderAdapterHostProcessError,
  ProviderAdapterProcessV1,
} from "@t3tools/provider-adapter";
import * as Deferred from "effect/Deferred";
import * as Effect from "effect/Effect";
import * as PlatformError from "effect/PlatformError";
import * as Ref from "effect/Ref";
import * as Sink from "effect/Sink";
import * as Stdio from "effect/Stdio";
import * as Stream from "effect/Stream";
import * as AcpClient from "effect-acp/client";
import * as AcpError from "effect-acp/errors";

const encoder = new TextEncoder();
const decoder = new TextDecoder();
const defaultStderrTailBytes = 16 * 1024;

export interface AcpHostConnectionOptions extends AcpClient.AcpClientOptions {
  /** Maximum number of recent stderr bytes retained for diagnostics. */
  readonly stderrTailBytes?: number;
}

export interface AcpHostConnection {
  /** The host-owned process. Session attachment and lifecycle decisions remain with the adapter. */
  readonly process: ProviderAdapterProcessV1;
  readonly client: AcpClient.AcpClient["Service"];
  readonly stderrTail: Effect.Effect<string>;
}

const platformError = (
  process: ProviderAdapterProcessV1,
  method: "read" | "write",
  cause: ProviderAdapterHostProcessError,
) =>
  PlatformError.systemError({
    _tag: "Unknown",
    module: "provider-adapter-acp",
    method,
    pathOrDescriptor: process.pid,
    description: cause.detail,
    cause,
  });

const appendTail = (current: Uint8Array, chunk: Uint8Array, maximumBytes: number): Uint8Array => {
  if (maximumBytes === 0) return new Uint8Array();
  if (chunk.length >= maximumBytes) return chunk.slice(chunk.length - maximumBytes);

  const retainedCurrent = current.slice(Math.max(0, current.length + chunk.length - maximumBytes));
  const next = new Uint8Array(retainedCurrent.length + chunk.length);
  next.set(retainedCurrent);
  next.set(chunk, retainedCurrent.length);
  return next;
};

const makeTerminationError = (
  process: ProviderAdapterProcessV1,
): Effect.Effect<AcpError.AcpError> =>
  Effect.match(process.exitCode, {
    onFailure: (cause) =>
      new AcpError.AcpTransportError({
        operation: "read-process-exit-status",
        pid: process.pid,
        cause,
      }),
    onSuccess: (code) => new AcpError.AcpProcessExitedError({ code, pid: process.pid }),
  });

export const make = Effect.fn("provider-adapter-acp/AcpHostConnection.make")(function* (
  process: ProviderAdapterProcessV1,
  options: AcpHostConnectionOptions = {},
) {
  const stderrTailBytes = Math.max(
    0,
    Math.floor(options.stderrTailBytes ?? defaultStderrTailBytes),
  );
  const stderrTail = yield* Ref.make<Uint8Array>(new Uint8Array());
  const writeFailure = yield* Deferred.make<never, PlatformError.PlatformError>();

  yield* process.stderr.pipe(
    Stream.runForEach((chunk) =>
      Ref.update(stderrTail, (current) => appendTail(current, chunk, stderrTailBytes)),
    ),
    Effect.ignore,
    Effect.forkScoped,
  );

  const stdin = process.stdout.pipe(
    Stream.mapError((cause) => platformError(process, "read", cause)),
    Stream.mergeEffect(Deferred.await(writeFailure)),
  );

  const stdio = Stdio.make({
    args: Effect.succeed([]),
    stdin,
    stdout: () =>
      Sink.forEach((chunk: string | Uint8Array) => {
        const bytes = typeof chunk === "string" ? encoder.encode(chunk) : chunk;
        return process.write(bytes).pipe(
          Effect.mapError((cause) => platformError(process, "write", cause)),
          Effect.tapError((error) => Deferred.fail(writeFailure, error).pipe(Effect.asVoid)),
        );
      }),
    stderr: () => Sink.drain,
  });

  // ACP sessions consume typed handlers, so retaining a raw copy of every notification would
  // only duplicate sustained large payloads. Callers can opt in when they need the raw stream.
  const client = yield* AcpClient.make(
    stdio,
    { ...options, captureRawNotifications: options.captureRawNotifications ?? false },
    makeTerminationError(process),
  );

  return {
    process,
    client,
    stderrTail: Ref.get(stderrTail).pipe(Effect.map((bytes) => decoder.decode(bytes))),
  } satisfies AcpHostConnection;
});
