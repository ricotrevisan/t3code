import { assert, it } from "@effect/vitest";
import * as Effect from "effect/Effect";
import * as Layer from "effect/Layer";
import * as Schema from "effect/Schema";
import * as SqlClient from "effect/unstable/sql/SqlClient";

import { runMigrations } from "../Migrations.ts";
import * as NodeSqliteClient from "@t3tools/shared/nodeSqliteClient";

const layer = it.layer(Layer.mergeAll(NodeSqliteClient.layerMemory()));
const encodeUnknownJson = Schema.encodeUnknownSync(Schema.fromJsonString(Schema.Unknown));

layer("044_BackfillProjectionThreadActivitySequence", (it) => {
  it.effect(
    "restores event order for existing activity rows without overwriting explicit order",
    () =>
      Effect.gen(function* () {
        const sql = yield* SqlClient.SqlClient;
        const threadId = "thread-activity-sequence-backfill";
        const createdAt = "2026-08-31T17:24:54.286Z";

        yield* runMigrations({ toMigrationInclusive: 43 });
        yield* sql`
        INSERT INTO projection_thread_activities (
          activity_id,
          thread_id,
          turn_id,
          tone,
          kind,
          summary,
          payload_json,
          sequence,
          created_at
        ) VALUES
          (
            'task-progress:thread-activity-sequence-backfill:task-1',
            ${threadId},
            NULL,
            'info',
            'task.progress',
            'Working',
            '{}',
            NULL,
            ${createdAt}
          ),
          (
            '000-terminal-task-1',
            ${threadId},
            NULL,
            'info',
            'task.completed',
            'Task completed',
            '{}',
            NULL,
            ${createdAt}
          ),
          (
            'explicit-sequence',
            ${threadId},
            NULL,
            'info',
            'task.progress',
            'Explicit',
            '{}',
            99,
            ${createdAt}
          ),
          (
            'legacy-without-event',
            ${threadId},
            NULL,
            'info',
            'task.progress',
            'Legacy',
            '{}',
            NULL,
            ${createdAt}
          )
      `;

        for (const [eventId, streamVersion, activityId] of [
          ["event-progress-1", 1, "task-progress:thread-activity-sequence-backfill:task-1"],
          ["event-progress-2", 2, "task-progress:thread-activity-sequence-backfill:task-1"],
          ["event-completed", 3, "000-terminal-task-1"],
          ["event-explicit", 4, "explicit-sequence"],
        ] as const) {
          yield* sql`
          INSERT INTO orchestration_events (
            event_id,
            aggregate_kind,
            stream_id,
            stream_version,
            event_type,
            occurred_at,
            command_id,
            causation_event_id,
            correlation_id,
            actor_kind,
            payload_json,
            metadata_json
          ) VALUES (
            ${eventId},
            'thread',
            ${threadId},
            ${streamVersion},
            'thread.activity-appended',
            ${createdAt},
            NULL,
            NULL,
            NULL,
            'provider',
            ${encodeUnknownJson({
              threadId,
              activity: { id: activityId },
            })},
            '{}'
          )
        `;
        }

        yield* runMigrations({ toMigrationInclusive: 44 });

        const rows = yield* sql<{
          readonly activityId: string;
          readonly sequence: number | null;
        }>`
        SELECT
          activity_id AS "activityId",
          sequence
        FROM projection_thread_activities
        WHERE thread_id = ${threadId}
        ORDER BY activity_id ASC
      `;

        assert.deepEqual(rows, [
          { activityId: "000-terminal-task-1", sequence: 3 },
          { activityId: "explicit-sequence", sequence: 99 },
          { activityId: "legacy-without-event", sequence: null },
          {
            activityId: "task-progress:thread-activity-sequence-backfill:task-1",
            sequence: 2,
          },
        ]);
      }),
  );
});
