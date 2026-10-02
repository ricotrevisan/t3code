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
import { inspectCleanupFiles } from "./threadCleanupFiles.ts";

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
        remoteBranch: { cwd: string; url: string; branch: string; head: string } | null;
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
    const verifyIntegrated = Effect.fn("ThreadCleanup.verifyIntegrated")(function* (
      cwd: string,
      head: string,
      base: string,
    ) {
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
    });
    const inspectRemoteBranch = Effect.fn("ThreadCleanup.inspectRemoteBranch")(function* (
      thread: OrchestrationThreadShell,
    ) {
      const project = (yield* snapshots.getShellSnapshot()).projects.find(
        (p) => p.id === thread.projectId,
      );
      if (!project || !thread.branch)
        return yield* new ThreadCleanupError({ message: "The project or branch is unavailable." });
      const cwd = project.workspaceRoot;
      const branch = thread.branch;
      const fetchUrl = yield* git.execute({
        operation: "ThreadCleanup.remoteUrl",
        cwd,
        args: ["remote", "get-url", "--all", "origin"],
      });
      const pushUrl = yield* git.execute({
        operation: "ThreadCleanup.pushUrl",
        cwd,
        args: ["remote", "get-url", "--push", "--all", "origin"],
      });
      const url = pushUrl.stdout.trim();
      if (
        !url ||
        url.includes("\n") ||
        url !== fetchUrl.stdout.trim() ||
        /github\.com[:/]pingdotgg\/t3code(?:\.git)?\/?$/i.test(url)
      )
        return yield* new ThreadCleanupError({
          message: "The origin push target needs manual review.",
        });
      const remoteHead = yield* git.execute({
        operation: "ThreadCleanup.remoteHead",
        cwd,
        args: ["ls-remote", "--symref", "--", url, "HEAD", `refs/heads/${branch}`],
      });
      const refs = remoteHead.stdout.trim().split("\n");
      const baseBranch = refs
        .find((line) => line.startsWith("ref: refs/heads/") && line.endsWith("\tHEAD"))
        ?.slice("ref: refs/heads/".length, -"\tHEAD".length);
      if (!baseBranch || branch === baseBranch || branch === "main" || branch === "master")
        return yield* new ThreadCleanupError({
          message: "The default or primary branch cannot be deleted.",
        });
      const head = refs.find((line) => line.endsWith(`\trefs/heads/${branch}`))?.split("\t")[0];
      if (!head)
        return yield* new ThreadCleanupError({
          message: "The remote branch has already been deleted or was never pushed.",
        });
      const local = yield* git.resolveCommit({ cwd, revision: `refs/heads/${branch}` });
      if (head !== local.commitSha)
        return yield* new ThreadCleanupError({
          message: "The remote branch differs from the local branch. Keep it for review.",
        });
      yield* git.fetchRemoteTrackingBranch({ cwd, remoteName: "origin", remoteBranch: baseBranch });
      const base = yield* git.resolveCommit({ cwd, revision: `refs/remotes/origin/${baseBranch}` });
      yield* verifyIntegrated(cwd, head, base.commitSha);
      return { cwd, url, branch, head };
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
      // A process started with git -C can use this checkout while its cwd is elsewhere.
      // Git resolves per-worktree and shared administration paths for us.
      const lockPaths = yield* git.execute({
        operation: "ThreadCleanup.operationLocks",
        cwd,
        args: [
          "rev-parse",
          ...[
            "index.lock",
            "HEAD.lock",
            "ORIG_HEAD.lock",
            "config.lock",
            "packed-refs.lock",
            "shallow.lock",
            `refs/heads/${thread.branch}.lock`,
          ].flatMap((lock) => ["--git-path", lock]),
        ],
      });
      if (lockPaths.stdoutTruncated)
        return yield* new ThreadCleanupError({ message: "Could not verify Git operation locks." });
      for (const lock of lockPaths.stdout.trim().split("\n")) {
        if (yield* fs.exists(path.resolve(cwd, lock)))
          return yield* new ThreadCleanupError({
            message: "A Git operation lock is present. Finish that operation and review again.",
          });
      }
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
      yield* verifyIntegrated(cwd, head, base);
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
      const remoteInspection = blocked
        ? null
        : yield* inspectRemoteBranch(thread).pipe(Effect.result);
      const remoteBranch = remoteInspection?._tag === "Success" ? remoteInspection.success : null;
      const remoteBlocked =
        blocked ??
        (remoteInspection?._tag === "Failure" ? failure(remoteInspection.failure).message : null);
      const unexpectedFiles = worktree?.files.length
        ? `Preserve these local files before removal: ${worktree.files.slice(0, 5).join(", ")}${worktree.files.length > 5 ? ` (and ${worktree.files.length - 5} more)` : ""}.`
        : null;
      const result: ThreadCleanupReview = {
        reviewId: NodeCrypto.randomUUID(),
        threadId,
        title: thread.title,
        reviewedAt: now,
        actions: [
          {
            id: "worktree",
            title: "Remove worktree",
            detail: worktree
              ? `${worktree.cwd}. Generated files are discarded. The local branch and conversation are kept.`
              : (thread.worktreePath ?? "This thread uses the project's local checkout."),
            blockedReason: worktreeBlocked ?? unexpectedFiles,
          },
          {
            id: "remote-branch",
            title: "Delete remote branch",
            detail: `origin/${thread.branch ?? "unknown"}. Delete only if fully merged and unchanged. Keep the local branch.`,
            blockedReason: remoteBlocked,
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
        remoteBranch,
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
            if (action.id === "remote-branch") {
              const checked = yield* inspectRemoteBranch(current);
              const expected = entry.remoteBranch;
              if (
                !expected ||
                checked.cwd !== expected.cwd ||
                checked.url !== expected.url ||
                checked.branch !== expected.branch ||
                checked.head !== expected.head ||
                !cleanupReviewStillCurrent(
                  entry.thread,
                  yield* readThread(input.threadId),
                  DateTime.formatIso(yield* DateTime.now),
                )
              )
                return {
                  id: action.id,
                  status: "deferred" as const,
                  detail: "The remote branch or push target changed. Review cleanup again.",
                };
              // The server enforces this comparison atomically, even if a push races our checks.
              yield* git.execute({
                operation: "ThreadCleanup.deleteRemoteBranch",
                cwd: checked.cwd,
                args: [
                  "push",
                  `--force-with-lease=refs/heads/${checked.branch}:${checked.head}`,
                  "--",
                  checked.url,
                  `:refs/heads/${checked.branch}`,
                ],
              });
              yield* gitManager.invalidateStatus(checked.cwd);
              return {
                id: action.id,
                status: "completed" as const,
                detail: `Deleted origin/${checked.branch}. Local branch kept.`,
              };
            }
            const checked = yield* inspectWorktree(current);
            if (checked.head !== entry.head || checked.files.length > 0)
              return {
                id: action.id,
                status: "deferred" as const,
                detail: "The worktree or ignored files changed. Review cleanup again.",
              };
            const final = yield* inspectWorktree(yield* readThread(input.threadId));
            if (
              !cleanupReviewStillCurrent(
                entry.thread,
                yield* readThread(input.threadId),
                DateTime.formatIso(yield* DateTime.now),
              ) ||
              final.head !== entry.head ||
              final.files.length > 0
            )
              return {
                id: action.id,
                status: "deferred" as const,
                detail: "The checkout changed during cleanup checks. Worktree kept.",
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
              detail: "Worktree removed. Local branch and conversation kept.",
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
const make = Effect.gen(function* () {
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
