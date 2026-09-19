import * as Effect from "effect/Effect";
import * as SqlClient from "effect/unstable/sql/SqlClient";

/**
 * Persists the exact external adapter package that owns a provider session.
 *
 * All three columns are nullable so rows and projections created before this
 * migration remain readable. Readers accept either a complete triple or no
 * triple and reject partial identities.
 */
export default Effect.gen(function* () {
  const sql = yield* SqlClient.SqlClient;

  const runtimeColumns = yield* sql<{ readonly name: string }>`
    PRAGMA table_info(provider_session_runtime)
  `;
  const runtimeNames = new Set(runtimeColumns.map((column) => column.name));
  if (!runtimeNames.has("adapter_package_id")) {
    yield* sql`ALTER TABLE provider_session_runtime ADD COLUMN adapter_package_id TEXT`;
  }
  if (!runtimeNames.has("adapter_package_version")) {
    yield* sql`ALTER TABLE provider_session_runtime ADD COLUMN adapter_package_version TEXT`;
  }
  if (!runtimeNames.has("adapter_package_protocol_version")) {
    yield* sql`
      ALTER TABLE provider_session_runtime
      ADD COLUMN adapter_package_protocol_version INTEGER
    `;
  }

  const projectionColumns = yield* sql<{ readonly name: string }>`
    PRAGMA table_info(projection_thread_sessions)
  `;
  const projectionNames = new Set(projectionColumns.map((column) => column.name));
  if (!projectionNames.has("adapter_package_id")) {
    yield* sql`ALTER TABLE projection_thread_sessions ADD COLUMN adapter_package_id TEXT`;
  }
  if (!projectionNames.has("adapter_package_version")) {
    yield* sql`ALTER TABLE projection_thread_sessions ADD COLUMN adapter_package_version TEXT`;
  }
  if (!projectionNames.has("adapter_package_protocol_version")) {
    yield* sql`
      ALTER TABLE projection_thread_sessions
      ADD COLUMN adapter_package_protocol_version INTEGER
    `;
  }
});
