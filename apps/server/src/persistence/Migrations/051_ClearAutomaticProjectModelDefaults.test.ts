import { assert, it } from "@effect/vitest";
import * as Effect from "effect/Effect";
import * as SqlClient from "effect/unstable/sql/SqlClient";
import * as NodeSqliteClient from "@t3tools/shared/nodeSqliteClient";

import { runMigrations } from "../Migrations.ts";

for (const source of ["fork48", "fork50", "upstream"] as const) {
  it.layer(NodeSqliteClient.layerMemory())(`051 upgrade from ${source}`, (it) => {
    it.effect("retains existing threads and installs upstream columns and pull request links", () =>
      Effect.gen(function* () {
        const sql = yield* SqlClient.SqlClient;
        yield* runMigrations({ toMigrationInclusive: source === "fork48" ? 47 : 49 });
        if (source !== "upstream") {
          yield* sql`
            INSERT INTO effect_sql_migrations (migration_id, name)
            VALUES (${source === "fork48" ? 48 : 50}, 'ClearAutomaticProjectModelDefaults')
          `;
        }
        yield* sql`
          INSERT INTO projection_threads (
            thread_id, project_id, title, model_selection_json, runtime_mode,
            created_at, updated_at, linked_pull_request_json
          ) VALUES (
            'existing-thread', 'project', 'Existing thread',
            '{"instanceId":"codex","model":"gpt-5.4"}', 'full-access',
            '2026-09-01T00:00:00.000Z', '2026-09-01T00:00:00.000Z',
            '{"repository":"acme/widgets","number":7,"url":"https://github.com/acme/widgets/pull/7"}'
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
        const links = yield* sql`
          SELECT host, repository, number
          FROM projection_thread_pull_requests WHERE thread_id = 'existing-thread'
        `;
        assert.deepEqual(links, [{ host: "github.com", repository: "acme/widgets", number: 7 }]);
        assert.deepEqual(yield* runMigrations(), []);
      }),
    );
  });
}
