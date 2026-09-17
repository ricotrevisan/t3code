import type {
  ProviderAdapterProcessSupervisorV1,
  ProviderAdapterProcessV1,
} from "@t3tools/provider-adapter";
import { ProviderAdapterHostProcessError } from "@t3tools/provider-adapter";
import type { ThreadId } from "@t3tools/contracts";
import { resolveSpawnCommand } from "@t3tools/shared/shell";
import { loadDirenvExportedEnv } from "../workspace/workspaceDirenvEnv.ts";
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

export interface ExternalProviderProcessExit {
  readonly pid: number;
  readonly threadId: ThreadId;
  readonly exitCode: number;
}

export interface ExternalProviderProcessSupervisor {
  readonly processes: ProviderAdapterProcessSupervisorV1;
  readonly unexpectedExits: Stream.Stream<ExternalProviderProcessExit>;
}

export const makeExternalProviderProcessSupervisor = Effect.fn(
  "makeExternalProviderProcessSupervisor",
)(function* (): Effect.fn.Return<
  ExternalProviderProcessSupervisor,
  never,
  ChildProcessSpawner.ChildProcessSpawner | Scope.Scope
> {
  const spawner = yield* ChildProcessSpawner.ChildProcessSpawner;
  const exits = yield* Queue.unbounded<ExternalProviderProcessExit, Cause.Done<void>>();
  yield* Effect.addFinalizer(() => Queue.end(exits));

  const spawn: ProviderAdapterProcessSupervisorV1["spawn"] = (input) =>
    Effect.gen(function* (): Effect.fn.Return<
      ProviderAdapterProcessV1,
      ProviderAdapterHostProcessError,
      Scope.Scope
    > {
      const args = input.args ?? [];
      // Adapter-owned processes inherit the project environment the same way first-party runtimes
      // do. The package cannot read the workspace, so the host resolves direnv here.
      const direnvEnv =
        input.cwd === undefined
          ? {}
          : loadDirenvExportedEnv(input.cwd, { env: input.environment ?? process.env });
      const environment =
        input.environment === undefined
          ? Object.keys(direnvEnv).length > 0
            ? { ...process.env, ...direnvEnv }
            : undefined
          : { ...input.environment, ...direnvEnv };
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
      const intentionalClose = yield* Ref.make(false);
      const attachedSessions = yield* Ref.make<ReadonlySet<ThreadId>>(
        input.purpose.kind === "session" ? new Set([input.purpose.threadId]) : new Set(),
      );
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
      yield* Effect.gen(function* () {
        const result = yield* child.exitCode.pipe(Effect.result);
        yield* Ref.set(closed, true);
        yield* Queue.end(outgoing);
        if (result._tag === "Success" && !(yield* Ref.get(intentionalClose))) {
          const owners = yield* Ref.get(attachedSessions);
          yield* Effect.forEach(owners, (threadId) =>
            Queue.offer(exits, {
              pid: Number(child.pid),
              threadId,
              exitCode: Number(result.success),
            }),
          );
        }
      }).pipe(Effect.forkScoped);

      const close = Effect.gen(function* () {
        yield* Ref.set(intentionalClose, true);
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
        attachSession: (threadId) =>
          Ref.update(attachedSessions, (current) => new Set([...current, threadId])),
        detachSession: (threadId) =>
          Ref.update(attachedSessions, (current) => {
            const next = new Set(current);
            next.delete(threadId);
            return next;
          }),
        expectExit: Ref.set(intentionalClose, true),
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

  return {
    processes: { spawn },
    unexpectedExits: Stream.fromQueue(exits),
  };
});
