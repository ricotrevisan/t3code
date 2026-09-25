import { assert, it } from "@effect/vitest";
import * as Effect from "effect/Effect";
import * as SqlClient from "effect/unstable/sql/SqlClient";
import * as NodeSqliteClient from "@t3tools/shared/nodeSqliteClient";

import { runMigrations } from "../Migrations.ts";
import repairAutoSettleDisabledAt from "./059_RepairForkAutoSettleDisabledAt.ts";

it.layer(NodeSqliteClient.layer({ filename: ":memory:" }))(
  "059_RepairForkAutoSettleDisabledAt",
  (it) => {
    it.effect("repairs a historical fork ledger already beyond upstream ID 54", () =>
      Effect.gen(function* () {
        const sql = yield* SqlClient.SqlClient;
        yield* runMigrations({ toMigrationInclusive: 53 });
        yield* sql`
          INSERT INTO effect_sql_migrations (migration_id, name) VALUES
            (54, 'RepairForkProjectionSchema'),
            (55, 'ProviderAdapterPackageSessionIdentity'),
            (56, 'PullRequestFilesViewed'),
            (57, 'ClearAutomaticProjectModelDefaultsForkCompatibility'),
            (58, 'RepairForkProjectionSchemaForkCompatibility')
        `;
        const columnsBefore = yield* sql<{
          readonly name: string;
        }>`PRAGMA table_info(projection_threads)`;
        assert.isFalse(columnsBefore.some((column) => column.name === "auto_settle_disabled_at"));
        const now = "2026-09-01T00:00:00.000Z";
        yield* sql`
          INSERT INTO projection_threads (thread_id, project_id, title, runtime_mode, created_at, updated_at)
          VALUES ('historical-thread', 'project', 'Existing thread', 'full-access', ${now}, ${now})
        `;
        assert.deepEqual(yield* runMigrations(), [[59, "RepairForkAutoSettleDisabledAt"]]);
        const rows = yield* sql<{ readonly autoSettleDisabledAt: string | null }>`
          SELECT auto_settle_disabled_at AS "autoSettleDisabledAt"
          FROM projection_threads WHERE thread_id = 'historical-thread'
        `;
        assert.deepEqual(rows, [{ autoSettleDisabledAt: null }]);
        yield* sql`UPDATE projection_threads SET auto_settle_disabled_at = ${now} WHERE thread_id = 'historical-thread'`;
        yield* repairAutoSettleDisabledAt;
        assert.deepEqual(
          yield* sql`
          SELECT auto_settle_disabled_at AS "autoSettleDisabledAt"
          FROM projection_threads WHERE thread_id = 'historical-thread'
        `,
          [{ autoSettleDisabledAt: now }],
        );
        assert.deepEqual(yield* runMigrations(), []);
      }),
    );
  },
);

it.layer(NodeSqliteClient.layer({ filename: ":memory:" }))(
  "059 on a fresh upstream schema",
  (it) => {
    it.effect("keeps the existing column and values", () =>
      Effect.gen(function* () {
        const sql = yield* SqlClient.SqlClient;
        yield* runMigrations({ toMigrationInclusive: 58 });
        const now = "2026-09-01T00:00:00.000Z";
        yield* sql`
          INSERT INTO projection_threads (thread_id, project_id, title, runtime_mode, created_at, updated_at, auto_settle_disabled_at)
          VALUES ('upstream-thread', 'project', 'Existing thread', 'full-access', ${now}, ${now}, ${now})
        `;
        assert.deepEqual(yield* runMigrations(), [[59, "RepairForkAutoSettleDisabledAt"]]);
        assert.deepEqual(
          yield* sql`
          SELECT auto_settle_disabled_at AS "autoSettleDisabledAt"
          FROM projection_threads WHERE thread_id = 'upstream-thread'
        `,
          [{ autoSettleDisabledAt: now }],
        );
      }),
    );
  },
);
