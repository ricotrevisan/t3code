import { it } from "@effect/vitest";
import { expect } from "vite-plus/test";
import * as Effect from "effect/Effect";
import * as Layer from "effect/Layer";
import * as Option from "effect/Option";
import * as Schema from "effect/Schema";
import * as SqlClient from "effect/unstable/sql/SqlClient";
import * as NodeSqliteClient from "@t3tools/shared/nodeSqliteClient";
import { CommandId, OrchestrationSession, ThreadId } from "@t3tools/contracts";

import { runMigrations } from "../Migrations.ts";
import { ProjectionThreadSessionRepositoryLive } from "../Layers/ProjectionThreadSessions.ts";
import { ProjectionThreadSessionRepository } from "../Services/ProjectionThreadSessions.ts";

const decodeSession = Schema.decodeEffect(OrchestrationSession);

it.layer(
  ProjectionThreadSessionRepositoryLive.pipe(
    Layer.provideMerge(NodeSqliteClient.layer({ filename: ":memory:" })),
  ),
)("correlated session-stop migration", (it) => {
  it.effect("migrates legacy sessions and roundtrips and clears the exact stop command", () =>
    Effect.gen(function* () {
      const sql = yield* SqlClient.SqlClient;
      yield* runMigrations({ toMigrationInclusive: 59 });
      const now = "2026-01-01T00:00:00.000Z";
      const threadId = ThreadId.make("legacy-session");
      yield* sql`
        INSERT INTO projection_thread_sessions
          (thread_id, status, provider_name, runtime_mode, active_turn_id, last_error, updated_at)
        VALUES (${threadId}, 'stopped', 'codex', 'approval-required', NULL, NULL, ${now})
      `;
      expect(yield* runMigrations()).toEqual([[60, "ProjectionThreadSessionStopCommandId"]]);
      const repository = yield* ProjectionThreadSessionRepository;
      const old = yield* repository.getByThreadId({ threadId }).pipe(Effect.map(Option.getOrThrow));
      expect(old.stopCommandId).toBeNull();
      const stopCommandId = CommandId.make("stop-origin-not-session-set-id");
      yield* repository.upsert({ ...old, stopCommandId });
      const stopped = yield* repository
        .getByThreadId({ threadId })
        .pipe(Effect.map(Option.getOrThrow));
      expect(stopped.stopCommandId).toBe(stopCommandId);
      const { stopCommandId: _marker, ...legacy } = stopped;
      yield* repository.upsert({ ...legacy, status: "ready" });
      const active = yield* repository
        .getByThreadId({ threadId })
        .pipe(Effect.map(Option.getOrThrow));
      expect(active.stopCommandId).toBeNull();
      expect(yield* runMigrations()).toEqual([]);
      const publicLegacy = { ...legacy, providerInstanceId: undefined };
      expect((yield* decodeSession(publicLegacy)).stopCommandId).toBeUndefined();
      expect(
        (yield* decodeSession({ ...publicLegacy, stopCommandId: null })).stopCommandId,
      ).toBeNull();
    }),
  );
});
