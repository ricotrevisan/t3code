import type {
  ProviderAdapterProcessSupervisorV1,
  ProviderAdapterProcessV1,
} from "@t3tools/provider-adapter";
import { ProviderAdapterHostProcessError } from "@t3tools/provider-adapter";
import { resolveSpawnCommand } from "@t3tools/shared/shell";
import * as Cause from "effect/Cause";
import * as Duration from "effect/Duration";
import * as Effect from "effect/Effect";
import * as Option from "effect/Option";
import * as Queue from "effect/Queue";
import * as Ref from "effect/Ref";
import type * as Scope from "effect/Scope";
import * as Stream from "effect/Stream";
import * as ChildProcess from "effect/unstable/process/ChildProcess";
import * as ChildProcessSpawner from "effect/unstable/process/ChildProcessSpawner";

const processError = (operation: string, detail: string, cause?: unknown) =>
  new ProviderAdapterHostProcessError({
    operation,
    detail,
    ...(cause === undefined ? {} : { cause }),
  });

export const makeExternalProviderProcessSupervisor = Effect.fn(
  "makeExternalProviderProcessSupervisor",
)(function* (): Effect.fn.Return<
  ProviderAdapterProcessSupervisorV1,
  never,
  ChildProcessSpawner.ChildProcessSpawner
> {
  const spawner = yield* ChildProcessSpawner.ChildProcessSpawner;

  const spawn: ProviderAdapterProcessSupervisorV1["spawn"] = (input) =>
    Effect.gen(function* (): Effect.fn.Return<
      ProviderAdapterProcessV1,
      ProviderAdapterHostProcessError,
      Scope.Scope
    > {
      const args = input.args ?? [];
      const environment = input.environment;
      const resolved = yield* resolveSpawnCommand(
        input.command,
        args,
        environment === undefined ? {} : { env: environment, extendEnv: true },
      ).pipe(
        Effect.mapError((cause) =>
          processError("resolve", `Could not resolve '${input.command}'.`, cause),
        ),
      );
      const outgoing = yield* Queue.bounded<Uint8Array, Cause.Done<void>>(64);
      const closed = yield* Ref.make(false);
      const child = yield* spawner
        .spawn(
          ChildProcess.make(resolved.command, resolved.args, {
            ...(input.cwd === undefined ? {} : { cwd: input.cwd }),
            ...(environment === undefined
              ? { extendEnv: true }
              : { env: environment, extendEnv: true }),
            shell: resolved.shell,
            stdin: { stream: Stream.fromQueue(outgoing), endOnDone: true },
            stdout: "pipe",
            stderr: "pipe",
            killSignal: "SIGTERM",
            forceKillAfter: Duration.seconds(2),
          }),
        )
        .pipe(
          Effect.mapError((cause) =>
            processError("spawn", `Could not start '${input.command}'.`, cause),
          ),
        );
      yield* child.exitCode.pipe(
        Effect.andThen(Ref.set(closed, true)),
        Effect.andThen(Queue.end(outgoing)),
        Effect.ignore,
        Effect.forkScoped,
      );

      const close = Effect.gen(function* () {
        if (yield* Ref.getAndSet(closed, true)) {
          return;
        }
        yield* Queue.end(outgoing);
        const exited = yield* child.exitCode.pipe(
          Effect.timeoutOption(Duration.seconds(2)),
          Effect.catch(() => Effect.succeedNone),
        );
        if (Option.isNone(exited)) {
          yield* child.kill().pipe(Effect.ignore);
        }
      });
      yield* Effect.addFinalizer(() => close.pipe(Effect.ignore));

      const write: ProviderAdapterProcessV1["write"] = (chunk) =>
        Effect.gen(function* () {
          if (yield* Ref.get(closed)) {
            return yield* processError("stdin", "The adapter process is closed.");
          }
          const running = yield* child.isRunning.pipe(
            Effect.mapError((cause) =>
              processError("stdin", "Could not inspect the adapter process.", cause),
            ),
          );
          if (!running) {
            yield* Ref.set(closed, true);
            yield* Queue.end(outgoing);
            return yield* processError("stdin", "The adapter process has exited.");
          }
          const accepted = yield* Queue.offer(outgoing, chunk);
          if (!accepted) {
            return yield* processError("stdin", "The adapter process stopped accepting input.");
          }
        });

      return {
        pid: Number(child.pid),
        stdout: child.stdout.pipe(
          Stream.mapError((cause) => processError("stdout", "Failed to read stdout.", cause)),
        ),
        stderr: child.stderr.pipe(
          Stream.mapError((cause) => processError("stderr", "Failed to read stderr.", cause)),
        ),
        exitCode: child.exitCode.pipe(
          Effect.map(Number),
          Effect.mapError((cause) =>
            processError("exitCode", "Failed to read the process exit code.", cause),
          ),
        ),
        write,
        close,
      };
    });

  return { spawn };
});
