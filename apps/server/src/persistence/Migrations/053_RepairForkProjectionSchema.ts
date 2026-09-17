import * as Effect from "effect/Effect";
import * as SqlClient from "effect/unstable/sql/SqlClient";

import migrateThreadPullRequests from "./050_ProjectionThreadPullRequests.ts";
import migrateThreadMessageContext from "./051_ProjectionThreadMessageContext.ts";

// Fork installs used IDs 51 and 52 before upstream added message context at 51, so their ledger
// sits above upstream's 52 (thread title state) and the runner skips that migration. Repair the
// historical IDs, then re-apply the skipped column here. SQLite has no ADD COLUMN IF NOT EXISTS,
// so the column add is guarded.
export default Effect.gen(function* () {
  const sql = yield* SqlClient.SqlClient;

  yield* migrateThreadPullRequests;
  yield* migrateThreadMessageContext;

  const columns = yield* sql<{ readonly name: string }>`
    PRAGMA table_info(projection_threads)
  `;

  if (!columns.some((column) => column.name === "title_state_json")) {
    yield* sql`
      ALTER TABLE projection_threads
      ADD COLUMN title_state_json TEXT
    `;
  }
});
