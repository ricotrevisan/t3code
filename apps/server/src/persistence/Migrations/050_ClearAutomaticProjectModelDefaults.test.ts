import { assert, it } from "@effect/vitest";
import * as Effect from "effect/Effect";
import * as SqlClient from "effect/unstable/sql/SqlClient";
import * as NodeSqliteClient from "@t3tools/shared/nodeSqliteClient";

import { runMigrations } from "../Migrations.ts";

for (const source of ["fork", "upstream"] as const) {
  it.layer(NodeSqliteClient.layerMemory())(`050 upgrade from ${source}`, (it) => {
    it.effect("retains existing threads and installs both upstream columns", () =>
      Effect.gen(function* () {
        const sql = yield* SqlClient.SqlClient;
        yield* runMigrations({ toMigrationInclusive: source === "fork" ? 47 : 49 });
        if (source === "fork") {
          yield* sql`
            INSERT INTO effect_sql_migrations (migration_id, name)
            VALUES (48, 'ClearAutomaticProjectModelDefaults')
          `;
        }
        yield* sql`
          INSERT INTO projection_threads (
            thread_id, project_id, title, model_selection_json, runtime_mode,
            created_at, updated_at
          ) VALUES (
            'existing-thread', 'project', 'Existing thread',
            '{"instanceId":"codex","model":"gpt-5.4"}', 'full-access',
            '2026-09-01T00:00:00.000Z', '2026-09-01T00:00:00.000Z'
          )
        `;
        yield* runMigrations();
        const rows = yield* sql`
          SELECT title, branch_pull_request_json, active_order_key
          FROM projection_threads WHERE thread_id = 'existing-thread'
        `;
        assert.deepEqual(rows, [
          {
            title: "Existing thread",
            branch_pull_request_json: null,
            active_order_key: null,
          },
        ]);
        assert.deepEqual(yield* runMigrations(), []);
      }),
    );
  });
}
