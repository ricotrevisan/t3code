import { ChildProcessSpawner } from "effect/unstable/process";
import * as NodeServices from "@effect/platform-node/NodeServices";
import { expect, it } from "@effect/vitest";
import * as Effect from "effect/Effect";
import * as FileSystem from "effect/FileSystem";
import * as Layer from "effect/Layer";
import * as Option from "effect/Option";
import * as Path from "effect/Path";
import {
  ProjectId,
  ThreadId,
  ProviderInstanceId,
  type OrchestrationThreadShell,
} from "@t3tools/contracts";
import * as ServerConfig from "./config.ts";
import * as Git from "./vcs/GitVcsDriver.ts";
import * as VcsProcess from "./vcs/VcsProcess.ts";
import { makeWith } from "./threadCleanup.ts";

const live = Git.layer.pipe(
  Layer.provide(ServerConfig.layerTest(process.cwd(), { prefix: "t3-cleanup-test-" })),
  Layer.provide(VcsProcess.layer),
  Layer.provideMerge(NodeServices.layer),
);
const NOW = "2026-01-01T00:00:00.000Z";
const fixture = Effect.gen(function* () {
  const fs = yield* FileSystem.FileSystem;
  const path = yield* Path.Path;
  const git = yield* Git.GitVcsDriver;
  const temporary = yield* fs.makeTempDirectoryScoped({ prefix: "t3-cleanup-" });
  const root = yield* fs.realPath(temporary);
  const main = path.join(root, "main");
  const worktree = path.join(root, "task");
  const remote = path.join(root, "remote.git");
  const home = path.join(root, "home");
  yield* fs.makeDirectory(main);
  const run = (cwd: string, args: string[]) =>
    git.execute({ operation: "cleanup.test", cwd, args });
  yield* run(main, ["init", "--initial-branch=main"]);
  yield* run(main, ["config", "user.name", "Test"]);
  yield* run(main, ["config", "user.email", "test@example.com"]);
  yield* fs.writeFileString(path.join(main, ".gitignore"), "notes/\nnode_modules/\n");
  yield* run(main, ["add", "."]);
  yield* run(main, ["commit", "-m", "Initial"]);
  yield* run(main, ["clone", "--bare", main, remote]);
  yield* run(main, ["remote", "add", "origin", remote]);
  yield* run(main, ["fetch", "origin"]);
  yield* run(main, ["remote", "set-head", "origin", "main"]);
  yield* run(main, ["worktree", "add", "-b", "task", worktree]);
  let thread: OrchestrationThreadShell = {
    id: ThreadId.make("cleanup-thread"),
    projectId: ProjectId.make("cleanup-project"),
    title: "Cleanup fixture",
    modelSelection: { instanceId: ProviderInstanceId.make("codex"), model: "gpt-5.4" },
    runtimeMode: "full-access",
    interactionMode: "default",
    branch: "task",
    worktreePath: worktree,
    pullRequests: [],
    latestTurn: null,
    createdAt: NOW,
    updatedAt: NOW,
    archivedAt: null,
    settledOverride: "settled",
    settledAt: NOW,
    session: null,
    latestUserMessageAt: NOW,
    hasPendingApprovals: false,
    hasPendingUserInput: false,
    hasActionableProposedPlan: false,
  };
  let others: OrchestrationThreadShell[] = [];
  let processCwd = main;
  const project = {
    id: thread.projectId,
    title: "Test",
    workspaceRoot: main,
    defaultModelSelection: null,
    scripts: [],
    createdAt: NOW,
    updatedAt: NOW,
  };
  const service = yield* makeWith({
    config: { baseDir: home },
    git,
    gitManager: { invalidateStatus: () => Effect.void },
    snapshots: {
      getThreadShellById: () =>
        Effect.succeed(thread.archivedAt ? Option.none() : Option.some(thread)),
      getShellSnapshot: () =>
        Effect.succeed({
          snapshotSequence: 1,
          projects: [project],
          threads: [thread, ...others],
          updatedAt: NOW,
        }),
      getArchivedShellSnapshot: () =>
        Effect.succeed({
          snapshotSequence: 1,
          projects: [],
          threads: thread.archivedAt ? [thread] : [],
          updatedAt: NOW,
        }),
    },
    engine: {
      dispatch: () =>
        Effect.sync(() => {
          thread = { ...thread, archivedAt: NOW };
          return { sequence: 2 };
        }),
    },
    providers: { listSessions: () => Effect.succeed([]) },
    terminals: {
      subscribeMetadata: (listener) =>
        listener({ type: "snapshot", terminals: [] }).pipe(Effect.as(() => {})),
    },
    processRunner: {
      run: () =>
        Effect.succeed({
          code: ChildProcessSpawner.ExitCode(0),
          signal: null,
          timedOut: false,
          stdoutInvalidUtf8: false,
          stderrInvalidUtf8: false,
          stdout: `p123\nn${processCwd}\n`,
          stderr: "",
          stdoutTruncated: false,
          stderrTruncated: false,
        }),
    },
  });
  return {
    fs,
    path,
    root,
    main,
    worktree,
    home,
    run,
    service,
    review: () => service.review({ threadId: thread.id }),
    execute: (reviewId: string) =>
      service.run({ threadId: thread.id, reviewId, selected: ["worktree", "archive"] }),
    update: (patch: Partial<OrchestrationThreadShell>) => {
      thread = { ...thread, ...patch };
    },
    share: () => {
      others = [{ ...thread, id: ThreadId.make("other"), title: "Other task" }];
    },
    process: () => {
      processCwd = worktree;
    },
    thread: () => thread,
  };
});

it.layer(live)("reviewed thread cleanup", (it) => {
  it.effect(
    "backs up ignored evidence, removes an integrated worktree, keeps its branch, and archives once",
    () =>
      Effect.gen(function* () {
        const f = yield* fixture;
        yield* f.fs.makeDirectory(f.path.join(f.worktree, "notes"));
        yield* f.fs.writeFileString(
          f.path.join(f.worktree, "notes", "evidence.md"),
          "Important evidence",
        );
        const review = yield* f.review();
        expect(review.actions.map((a) => a.blockedReason)).toEqual([null, null]);
        const result = yield* f.execute(review.reviewId);
        expect(result.actions.map((a) => a.status)).toEqual(["completed", "completed"]);
        expect(yield* f.fs.exists(f.worktree)).toBe(false);
        expect(
          yield* f.fs.readFileString(
            f.path.join(
              f.home,
              "cleanup-backups",
              review.reviewId,
              "files",
              "notes",
              "evidence.md",
            ),
          ),
        ).toBe("Important evidence");
        yield* f.run(f.main, ["show-ref", "--verify", "refs/heads/task"]);
        expect(yield* f.execute(review.reviewId)).toEqual(result);
      }),
  );
  it.effect("defers selected actions when a thread resumes after review", () =>
    Effect.gen(function* () {
      const f = yield* fixture;
      const review = yield* f.review();
      f.update({ settledOverride: "active", settledAt: null });
      const result = yield* f.execute(review.reviewId);
      expect(result.actions.map((a) => a.status)).toEqual(["deferred", "deferred"]);
      expect(yield* f.fs.exists(f.worktree)).toBe(true);
      expect(f.thread().archivedAt).toBeNull();
    }),
  );
  it.effect("protects shared worktrees, locks, and external processes", () =>
    Effect.gen(function* () {
      const shared = yield* fixture;
      shared.share();
      expect((yield* shared.review()).actions[0]?.blockedReason).toContain("Shared with");
      const locked = yield* fixture;
      yield* locked.run(locked.main, ["worktree", "lock", locked.worktree]);
      expect((yield* locked.review()).actions[0]?.blockedReason).toContain("locked");
      const busy = yield* fixture;
      busy.process();
      expect((yield* busy.review()).actions[0]?.blockedReason).toContain("running process");
    }),
  );
  it.effect("preserves clean worktrees with per-worktree or shared Git operation locks", () =>
    Effect.gen(function* () {
      for (const lock of ["index.lock", "HEAD.lock", "refs/heads/task.lock"]) {
        const f = yield* fixture;
        const review = yield* f.review();
        const location = yield* f.run(f.worktree, ["rev-parse", "--git-path", lock]);
        const absolute = f.path.resolve(f.worktree, location.stdout.trim());
        yield* f.fs.makeDirectory(f.path.dirname(absolute), { recursive: true });
        yield* f.fs.writeFileString(absolute, "held by another Git operation");
        expect((yield* f.review()).actions[0]?.blockedReason).toContain("Git operation lock");
        const result = yield* f.service.run({
          threadId: f.thread().id,
          reviewId: review.reviewId,
          selected: ["worktree"],
        });
        expect(result.actions[0]?.status).toBe("failed");
        expect(yield* f.fs.exists(f.worktree)).toBe(true);
        expect(yield* f.fs.exists(absolute)).toBe(true);
      }
    }),
  );
  it.effect("accepts squash integration but protects commits added after that merge", () =>
    Effect.gen(function* () {
      const f = yield* fixture;
      yield* f.fs.writeFileString(f.path.join(f.worktree, "feature.txt"), "Feature");
      yield* f.run(f.worktree, ["add", "."]);
      yield* f.run(f.worktree, ["commit", "-m", "Feature"]);
      expect((yield* f.review()).actions[0]?.blockedReason).toContain("not integrated");
      yield* f.run(f.main, ["merge", "--squash", "task"]);
      yield* f.run(f.main, ["commit", "-m", "Squashed feature"]);
      yield* f.run(f.main, ["push", "origin", "main"]);
      expect((yield* f.review()).actions[0]?.blockedReason).toBeNull();
      yield* f.fs.writeFileString(f.path.join(f.worktree, "feature.txt"), "Later work");
      yield* f.run(f.worktree, ["add", "."]);
      yield* f.run(f.worktree, ["commit", "-m", "After merge"]);
      expect((yield* f.review()).actions[0]?.blockedReason).toContain("not integrated");
    }),
  );
  it.effect("preserves changes made after the user reviewed the worktree", () =>
    Effect.gen(function* () {
      const f = yield* fixture;
      const review = yield* f.review();
      yield* f.fs.writeFileString(f.path.join(f.worktree, "uncommitted.txt"), "Keep this");
      const result = yield* f.service.run({
        threadId: f.thread().id,
        reviewId: review.reviewId,
        selected: ["worktree"],
      });
      expect(result.actions[0]?.status).toBe("failed");
      expect(yield* f.fs.readFileString(f.path.join(f.worktree, "uncommitted.txt"))).toBe(
        "Keep this",
      );
    }),
  );
});
