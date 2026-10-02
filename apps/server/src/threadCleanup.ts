import * as Clock from "effect/Clock";
import * as DateTime from "effect/DateTime";
import * as NodeCrypto from "node:crypto";
import {
  CommandId,
  ThreadCleanupError,
  type ThreadCleanupReview,
  type ThreadCleanupResult,
  type ThreadCleanupRunInput,
  type ThreadId,
  type OrchestrationThreadShell,
  type TerminalSummary,
} from "@t3tools/contracts";
import * as Context from "effect/Context";
import * as Effect from "effect/Effect";
import * as Layer from "effect/Layer";
import * as FileSystem from "effect/FileSystem";
import * as Path from "effect/Path";
import * as Semaphore from "effect/Semaphore";
import * as Option from "effect/Option";
import { ServerConfig } from "./config.ts";
import { GitVcsDriver } from "./vcs/GitVcsDriver.ts";
import { GitManager } from "./git/GitManager.ts";
import { ProjectionSnapshotQuery } from "./orchestration/Services/ProjectionSnapshotQuery.ts";
import { OrchestrationEngineService } from "./orchestration/Services/OrchestrationEngine.ts";
import { ProviderService } from "./provider/Services/ProviderService.ts";
import { TerminalManager } from "./terminal/Manager.ts";
import { ProcessRunner } from "./processRunner.ts";
import { withWorkspaceLease } from "./workspace/workspaceLease.ts";
import { cleanupReviewStillCurrent, threadCleanupBlockReason } from "./threadCleanupPolicy.ts";
import {
  backupCleanupFiles,
  inspectCleanupFiles,
  sameCleanupFiles,
  type CleanupFile,
} from "./threadCleanupFiles.ts";

export class ThreadCleanup extends Context.Service<
  ThreadCleanup,
  {
    readonly review: (input: {
      readonly threadId: ThreadId;
    }) => Effect.Effect<ThreadCleanupReview, ThreadCleanupError>;
    readonly run: (
      input: ThreadCleanupRunInput,
    ) => Effect.Effect<ThreadCleanupResult, ThreadCleanupError>;
  }
>()("t3/threadCleanup") {}

const failure = (error: unknown) =>
  new ThreadCleanupError({
    message: error instanceof Error ? error.message : "Cleanup could not be checked. Try again.",
  });

/** The orchestration boundary is narrow so cleanup can be exercised against real Git fixtures. */
export const makeWith = (dependencies: {
  config: Pick<ServerConfig["Service"], "baseDir">;
  git: GitVcsDriver["Service"];
  gitManager: Pick<GitManager["Service"], "invalidateStatus">;
  snapshots: Pick<
    ProjectionSnapshotQuery["Service"],
    "getThreadShellById" | "getShellSnapshot" | "getArchivedShellSnapshot"
  >;
  engine: Pick<OrchestrationEngineService["Service"], "dispatch">;
  providers: Pick<ProviderService["Service"], "listSessions">;
  terminals: Pick<TerminalManager["Service"], "subscribeMetadata">;
  processRunner: ProcessRunner["Service"];
}) =>
  Effect.gen(function* () {
    const { config, git, gitManager, snapshots, engine, providers, terminals, processRunner } =
      dependencies;
    const fs = yield* FileSystem.FileSystem;
    const path = yield* Path.Path;
    const serial = Semaphore.makeUnsafe(1);
    const reviews = new Map<
      string,
      {
        review: ThreadCleanupReview;
        thread: OrchestrationThreadShell;
        head: string | null;
        files: readonly CleanupFile[];
        result?: ThreadCleanupResult;
      }
    >();
    const inside = (root: string, target: string) => {
      const relative = path.relative(root, target);
      return (
        relative === "" ||
        (!path.isAbsolute(relative) && relative !== ".." && !relative.startsWith(`..${path.sep}`))
      );
    };
    const readThread = Effect.fn("ThreadCleanup.readThread")(function* (threadId: ThreadId) {
      const thread = yield* snapshots.getThreadShellById(threadId);
      if (Option.isNone(thread))
        return yield* new ThreadCleanupError({ message: "Thread no longer exists." });
      return thread.value;
    });
    const ignoredFiles = Effect.fn("ThreadCleanup.ignoredFiles")(function* (cwd: string) {
      const result = yield* git.execute({
        operation: "ThreadCleanup.ignoredFiles",
        cwd,
        args: ["ls-files", "--others", "--ignored", "--exclude-standard", "--directory", "-z"],
        maxOutputBytes: 1024 * 1024,
      });
      if (result.stdoutTruncated)
        return yield* new ThreadCleanupError({
          message: "The ignored-file list is too large. Preserve local files manually first.",
        });
      return yield* Effect.tryPromise({
        try: () => inspectCleanupFiles(cwd, result.stdout.split("\0").filter(Boolean)),
        catch: failure,
      });
    });
    const inspectWorktree = Effect.fn("ThreadCleanup.inspectWorktree")(function* (
      thread: OrchestrationThreadShell,
    ) {
      if (!thread.worktreePath || !thread.branch)
        return yield* new ThreadCleanupError({
          message: "This thread uses a local checkout, not a removable worktree.",
        });
      const cwd = path.resolve(thread.worktreePath);
      if (!(yield* fs.exists(cwd)))
        return yield* new ThreadCleanupError({
          message: "This worktree has already been removed.",
        });
      if ((yield* fs.realPath(cwd)) !== cwd)
        return yield* new ThreadCleanupError({
          message: "The worktree path is a symbolic link. Keep it for manual review.",
        });
      const serverHome = yield* fs
        .realPath(config.baseDir)
        .pipe(Effect.orElseSucceed(() => path.resolve(config.baseDir)));
      if (inside(cwd, serverHome))
        return yield* new ThreadCleanupError({
          message:
            "This worktree contains the running server's data or cleanup backups. Keep it for manual cleanup.",
        });
      const snapshot = yield* snapshots.getShellSnapshot();
      const archived = yield* snapshots.getArchivedShellSnapshot();
      const project = snapshot.projects.find((p) => p.id === thread.projectId);
      if (!project)
        return yield* new ThreadCleanupError({ message: "The thread's project is unavailable." });
      for (const other of [...snapshot.threads, ...archived.threads]) {
        if (other.id === thread.id) continue;
        const root =
          other.worktreePath ??
          snapshot.projects.find((p) => p.id === other.projectId)?.workspaceRoot;
        if (
          root &&
          inside(cwd, yield* fs.realPath(root).pipe(Effect.orElseSucceed(() => path.resolve(root))))
        )
          return yield* new ThreadCleanupError({
            message: `Shared with “${other.title}”. Keep this worktree until that thread no longer uses it.`,
          });
      }
      for (const p of snapshot.projects) {
        const root = yield* fs
          .realPath(p.workspaceRoot)
          .pipe(Effect.orElseSucceed(() => path.resolve(p.workspaceRoot)));
        if (inside(cwd, root))
          return yield* new ThreadCleanupError({
            message: "A project uses this checkout as its root. It cannot be removed.",
          });
      }
      if ((yield* fs.stat(path.join(cwd, ".git"))).type !== "File")
        return yield* new ThreadCleanupError({ message: "Primary checkouts cannot be removed." });
      const sessions = yield* providers.listSessions();
      if (
        sessions.some(
          (s) =>
            s.status !== "closed" &&
            (s.threadId === thread.id || (s.cwd && inside(cwd, path.resolve(s.cwd)))),
        )
      )
        return yield* new ThreadCleanupError({
          message: "An agent session still uses this worktree.",
        });
      let terminalList: readonly TerminalSummary[] = [];
      const unsubscribe = yield* terminals.subscribeMetadata((event) =>
        Effect.sync(() => {
          if (event.type === "snapshot") terminalList = event.terminals;
        }),
      );
      unsubscribe();
      if (
        terminalList.some(
          (t) =>
            (t.status === "starting" || t.status === "running") && inside(cwd, path.resolve(t.cwd)),
        )
      )
        return yield* new ThreadCleanupError({
          message: "A terminal or dev server still uses this worktree.",
        });
      // External dev servers and shells can outlive their T3 terminal. Never kill them.
      const processes = yield* processRunner.run({
        command: "lsof",
        args: ["-d", "cwd", "-Fpn"],
        cwd: project.workspaceRoot,
        timeout: "10 seconds",
        maxOutputBytes: 2 * 1024 * 1024,
      });
      if (processes.code !== 0 || processes.stdoutTruncated)
        return yield* new ThreadCleanupError({
          message: "Could not verify running processes. Keep the worktree for manual cleanup.",
        });
      if (
        processes.stdout
          .split("\n")
          .some((line) => line.startsWith("n") && inside(cwd, line.slice(1)))
      )
        return yield* new ThreadCleanupError({
          message: "A running process still uses this worktree. Stop it and review again.",
        });
      const status = yield* git.statusDetailsLocal(cwd);
      if (!status.isRepo || status.branch !== thread.branch || status.hasWorkingTreeChanges)
        return yield* new ThreadCleanupError({
          message: "The worktree has local changes or its branch changed. Keep it for review.",
        });
      const listing = yield* git.execute({
        operation: "ThreadCleanup.worktreeLocks",
        cwd,
        args: ["worktree", "list", "--porcelain", "-z"],
      });
      const ownRecord = listing.stdout
        .split("\0\0")
        .find((record) => record.startsWith(`worktree ${cwd}\0`));
      if (
        !ownRecord ||
        ownRecord.split("\0").some((line) => line === "locked" || line.startsWith("locked "))
      )
        return yield* new ThreadCleanupError({
          message: "This worktree is locked or no longer registered.",
        });
      const head = (yield* git.resolveCommit({ cwd, revision: "HEAD" })).commitSha;
      const remote = yield* git.resolvePrimaryRemoteName(project.workspaceRoot);
      const branch = yield* git.resolveDefaultBranchName(project.workspaceRoot, remote);
      if (!branch)
        return yield* new ThreadCleanupError({ message: "Could not identify the base branch." });
      yield* git.fetchRemoteTrackingBranch({
        cwd: project.workspaceRoot,
        remoteName: remote,
        remoteBranch: branch,
      });
      const base = (yield* git.resolveCommit({ cwd, revision: `refs/remotes/${remote}/${branch}` }))
        .commitSha;
      const ancestor = yield* git.execute({
        operation: "ThreadCleanup.integrated",
        cwd,
        args: ["merge-base", "--is-ancestor", head, base],
        allowNonZeroExit: true,
      });
      if (ancestor.exitCode !== 0) {
        // A squash merge need not retain ancestry. Merging the branch into the current
        // base must produce exactly the base tree, without conflicts or extra changes.
        const merge = yield* git.execute({
          operation: "ThreadCleanup.squashIntegrated",
          cwd,
          args: ["merge-tree", "--write-tree", base, head],
          allowNonZeroExit: true,
        });
        const baseTree = yield* git.execute({
          operation: "ThreadCleanup.baseTree",
          cwd,
          args: ["rev-parse", `${base}^{tree}`],
        });
        if (merge.exitCode !== 0 || merge.stdout.trim() !== baseTree.stdout.trim())
          return yield* new ThreadCleanupError({
            message:
              "The entire branch is not integrated into the base branch, including commits added after a merge.",
          });
      }
      const files = yield* ignoredFiles(cwd);
      return { cwd, projectRoot: project.workspaceRoot, head, files };
    });
    const review = Effect.fn("ThreadCleanup.review")(function* ({
      threadId,
    }: {
      readonly threadId: ThreadId;
    }) {
      const now = DateTime.formatIso(yield* DateTime.now);
      const thread = yield* readThread(threadId);
      const blocked = threadCleanupBlockReason(thread, now);
      const inspected = blocked ? null : yield* inspectWorktree(thread).pipe(Effect.result);
      const worktree = inspected?._tag === "Success" ? inspected.success : null;
      const worktreeBlocked =
        blocked ?? (inspected?._tag === "Failure" ? failure(inspected.failure).message : null);
      const result: ThreadCleanupReview = {
        reviewId: NodeCrypto.randomUUID(),
        threadId,
        title: thread.title,
        reviewedAt: now,
        actions: [
          {
            id: "worktree",
            title: "Back up local files and remove the worktree",
            detail: worktree
              ? `${worktree.cwd}. ${worktree.files.length} ignored ${worktree.files.length === 1 ? "file" : "files"} will be backed up and verified. Branches and conversation are kept.`
              : (thread.worktreePath ?? "This thread uses the project's local checkout."),
            blockedReason: worktreeBlocked,
          },
          {
            id: "archive",
            title: "Archive this thread",
            detail: "Hide it from the settled list. Keep the conversation and allow reopening.",
            blockedReason: blocked,
          },
        ],
      };
      for (const [id, entry] of reviews)
        if (Date.parse(entry.review.reviewedAt) < (yield* Clock.currentTimeMillis) - 15 * 60_000)
          reviews.delete(id);
      if (reviews.size >= 200) reviews.delete(reviews.keys().next().value!);
      reviews.set(result.reviewId, {
        review: result,
        thread,
        head: worktree?.head ?? null,
        files: worktree?.files ?? [],
      });
      return result;
    }, Effect.mapError(failure));
    const run = Effect.fn("ThreadCleanup.run")(
      function* (input: ThreadCleanupRunInput) {
        const entry = reviews.get(input.reviewId);
        if (
          !entry ||
          entry.thread.id !== input.threadId ||
          Date.parse(entry.review.reviewedAt) < (yield* Clock.currentTimeMillis) - 15 * 60_000
        )
          return yield* new ThreadCleanupError({
            message: "This review expired. Review cleanup again before running it.",
          });
        if (entry.result) return entry.result;
        const actions: Array<ThreadCleanupResult["actions"][number]> = [];
        for (const action of entry.review.actions) {
          if (!input.selected.includes(action.id)) continue;
          if (action.blockedReason) {
            actions.push({ id: action.id, status: "deferred", detail: action.blockedReason });
            continue;
          }
          const execute = Effect.gen(function* () {
            const current = yield* readThread(input.threadId);
            if (
              !cleanupReviewStillCurrent(
                entry.thread,
                current,
                DateTime.formatIso(yield* DateTime.now),
              )
            )
              return {
                id: action.id,
                status: "deferred" as const,
                detail: "The thread changed or resumed. Review cleanup again.",
              };
            if (action.id === "archive") {
              yield* engine.dispatch({
                type: "thread.archive",
                commandId: CommandId.make(`cleanup:${input.reviewId}:archive`),
                threadId: input.threadId,
                onlyIfSettledAt: entry.thread.settledAt!,
              });
              const archived = (yield* snapshots.getArchivedShellSnapshot()).threads.find(
                (thread) => thread.id === input.threadId,
              );
              return {
                id: action.id,
                status: archived?.archivedAt ? ("completed" as const) : ("deferred" as const),
                detail: archived?.archivedAt
                  ? "Archived. Conversation preserved."
                  : "The thread resumed before it could be archived.",
              };
            }
            const checked = yield* inspectWorktree(current);
            if (checked.head !== entry.head || !sameCleanupFiles(checked.files, entry.files))
              return {
                id: action.id,
                status: "deferred" as const,
                detail: "The worktree or ignored files changed. Review cleanup again.",
              };
            const backup = path.join(config.baseDir, "cleanup-backups", input.reviewId, "files");
            if (checked.files.length)
              yield* Effect.tryPromise({
                try: () => backupCleanupFiles(checked.cwd, backup, checked.files),
                catch: failure,
              });
            const final = yield* inspectWorktree(yield* readThread(input.threadId));
            if (
              !cleanupReviewStillCurrent(
                entry.thread,
                yield* readThread(input.threadId),
                DateTime.formatIso(yield* DateTime.now),
              ) ||
              final.head !== entry.head ||
              !sameCleanupFiles(final.files, entry.files)
            )
              return {
                id: action.id,
                status: "deferred" as const,
                detail: `The checkout changed during backup. Worktree kept.${checked.files.length ? ` Backup: ${backup}` : ""}`,
              };
            yield* git.removeWorktree({
              cwd: checked.projectRoot,
              path: checked.cwd,
              force: false,
            });
            yield* gitManager.invalidateStatus(checked.projectRoot);
            return {
              id: action.id,
              status: "completed" as const,
              detail: `Worktree removed; branches kept.${checked.files.length ? ` Verified backup: ${backup}` : ""}`,
            };
          });
          const result = yield* (
            action.id === "worktree" && entry.thread.worktreePath
              ? withWorkspaceLease(path.resolve(entry.thread.worktreePath), execute)
              : execute
          ).pipe(
            Effect.catch((error) =>
              Effect.succeed({
                id: action.id,
                status: "failed" as const,
                detail: failure(error).message,
              }),
            ),
          );
          actions.push(result);
        }
        const result = { threadId: input.threadId, actions };
        entry.result = result;
        return result;
      },
      Effect.mapError(failure),
      (effect) => serial.withPermit(effect),
      Effect.uninterruptible,
    );
    return { review, run };
  });
export const make = Effect.gen(function* () {
  return yield* makeWith({
    config: yield* ServerConfig,
    git: yield* GitVcsDriver,
    gitManager: yield* GitManager,
    snapshots: yield* ProjectionSnapshotQuery,
    engine: yield* OrchestrationEngineService,
    providers: yield* ProviderService,
    terminals: yield* TerminalManager,
    processRunner: yield* ProcessRunner,
  });
});
export const layer = Layer.effect(ThreadCleanup, make);
