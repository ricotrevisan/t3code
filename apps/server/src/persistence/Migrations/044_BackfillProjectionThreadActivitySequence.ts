import * as Effect from "effect/Effect";
import * as SqlClient from "effect/unstable/sql/SqlClient";

export default Effect.gen(function* () {
  const sql = yield* SqlClient.SqlClient;

  yield* sql`
    WITH latest_activity_events AS (
      SELECT
        stream_id AS thread_id,
        json_extract(payload_json, '$.activity.id') AS activity_id,
        MAX(sequence) AS sequence
      FROM orchestration_events
      WHERE event_type = 'thread.activity-appended'
      GROUP BY
        stream_id,
        json_extract(payload_json, '$.activity.id')
    )
    UPDATE projection_thread_activities AS activity
    SET sequence = latest.sequence
    FROM latest_activity_events AS latest
    WHERE activity.sequence IS NULL
      AND latest.activity_id IS NOT NULL
      AND activity.thread_id = latest.thread_id
      AND activity.activity_id = latest.activity_id
  `;
});
