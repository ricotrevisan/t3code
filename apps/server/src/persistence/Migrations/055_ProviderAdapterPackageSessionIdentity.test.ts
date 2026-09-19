import { assert, it } from "@effect/vitest";
import * as Effect from "effect/Effect";
import * as Layer from "effect/Layer";
import * as SqlClient from "effect/unstable/sql/SqlClient";

import { runMigrations } from "../Migrations.ts";
import * as NodeSqliteClient from "@t3tools/shared/nodeSqliteClient";

const layer = it.layer(Layer.mergeAll(NodeSqliteClient.layer({ filename: ":memory:" })));

layer("055_ProviderAdapterPackageSessionIdentity", (it) => {
  it.effect("completes partial upgrades and preserves legacy rows", () =>
    Effect.gen(function* () {
      const sql = yield* SqlClient.SqlClient;

      yield* runMigrations({ toMigrationInclusive: 41 });
      yield* sql`
        INSERT INTO provider_session_runtime (
          thread_id,
          provider_name,
          provider_instance_id,
          adapter_key,
          runtime_mode,
          status,
          last_seen_at
        ) VALUES (
          'thread-runtime-legacy',
          'codex',
          'codex',
          'codex',
          'full-access',
          'running',
          '2026-01-01T00:00:00.000Z'
        )
      `;
      yield* sql`
        INSERT INTO projection_thread_sessions (
          thread_id,
          status,
          provider_name,
          provider_instance_id,
          runtime_mode,
          updated_at
        ) VALUES (
          'thread-projection-legacy',
          'ready',
          'codex',
          'codex',
          'full-access',
          '2026-01-01T00:00:00.000Z'
        )
      `;

      yield* sql`
        ALTER TABLE provider_session_runtime
        ADD COLUMN adapter_package_id TEXT
      `;
      yield* sql`
        ALTER TABLE projection_thread_sessions
        ADD COLUMN adapter_package_version TEXT
      `;

      yield* runMigrations({ toMigrationInclusive: 55 });

      for (const table of ["provider_session_runtime", "projection_thread_sessions"] as const) {
        const columns =
          table === "provider_session_runtime"
            ? yield* sql<{ readonly name: string }>`PRAGMA table_info(provider_session_runtime)`
            : yield* sql<{ readonly name: string }>`PRAGMA table_info(projection_thread_sessions)`;
        const names = new Set(columns.map((column) => column.name));
        assert.ok(names.has("adapter_package_id"));
        assert.ok(names.has("adapter_package_version"));
        assert.ok(names.has("adapter_package_protocol_version"));
      }

      const runtimeRows = yield* sql<{
        readonly adapter_package_id: string | null;
        readonly adapter_package_version: string | null;
        readonly adapter_package_protocol_version: number | null;
      }>`
        SELECT
          adapter_package_id,
          adapter_package_version,
          adapter_package_protocol_version
        FROM provider_session_runtime
        WHERE thread_id = 'thread-runtime-legacy'
      `;
      const projectionRows = yield* sql<{
        readonly adapter_package_id: string | null;
        readonly adapter_package_version: string | null;
        readonly adapter_package_protocol_version: number | null;
      }>`
        SELECT
          adapter_package_id,
          adapter_package_version,
          adapter_package_protocol_version
        FROM projection_thread_sessions
        WHERE thread_id = 'thread-projection-legacy'
      `;
      assert.deepStrictEqual(runtimeRows, [
        {
          adapter_package_id: null,
          adapter_package_version: null,
          adapter_package_protocol_version: null,
        },
      ]);
      assert.deepStrictEqual(projectionRows, [
        {
          adapter_package_id: null,
          adapter_package_version: null,
          adapter_package_protocol_version: null,
        },
      ]);
    }),
  );
});
