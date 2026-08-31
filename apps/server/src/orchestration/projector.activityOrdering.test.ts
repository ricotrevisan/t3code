import { assert, it } from "@effect/vitest";
import {
  CommandId,
  EventId,
  ProjectId,
  ProviderDriverKind,
  ThreadId,
  type OrchestrationEvent,
} from "@t3tools/contracts";
import * as Effect from "effect/Effect";

import { createEmptyReadModel, projectEvent } from "./projector.ts";

const createdAt = "2026-08-31T17:24:54.286Z";
const threadId = ThreadId.make("thread-equal-timestamp-task-order");

function event(
  sequence: number,
  type: OrchestrationEvent["type"],
  payload: unknown,
): OrchestrationEvent {
  return {
    sequence,
    eventId: EventId.make(`event-${sequence}`),
    type,
    aggregateKind: "thread",
    aggregateId: threadId,
    occurredAt: createdAt,
    commandId: CommandId.make(`command-${sequence}`),
    causationEventId: null,
    correlationId: null,
    metadata: {},
    payload: payload as never,
  } as OrchestrationEvent;
}

it.effect("uses event sequence to order unsequenced activities with equal timestamps", () =>
  Effect.gen(function* () {
    let readModel = yield* projectEvent(
      createEmptyReadModel(createdAt),
      event(1, "thread.created", {
        threadId,
        projectId: ProjectId.make("project-1"),
        title: "demo",
        modelSelection: {
          provider: ProviderDriverKind.make("codex"),
          model: "gpt-5-codex",
        },
        runtimeMode: "full-access",
        branch: null,
        worktreePath: null,
        createdAt,
        updatedAt: createdAt,
      }),
    );
    readModel = yield* projectEvent(
      readModel,
      event(2, "thread.activity-appended", {
        threadId,
        activity: {
          id: EventId.make(`task-progress:${threadId}:task-1`),
          tone: "info",
          kind: "task.progress",
          summary: "Working",
          payload: { taskId: "task-1", status: "running", agentKind: "agent" },
          turnId: null,
          createdAt,
        },
      }),
    );
    readModel = yield* projectEvent(
      readModel,
      event(3, "thread.activity-appended", {
        threadId,
        activity: {
          id: EventId.make("000-terminal-task-1"),
          tone: "info",
          kind: "task.completed",
          summary: "Task completed",
          payload: { taskId: "task-1", status: "completed", agentKind: "agent" },
          turnId: null,
          createdAt,
        },
      }),
    );

    assert.deepEqual(
      readModel.threads[0]?.activities.map((activity) => ({
        id: activity.id,
        sequence: activity.sequence,
      })),
      [
        { id: EventId.make(`task-progress:${threadId}:task-1`), sequence: 2 },
        { id: EventId.make("000-terminal-task-1"), sequence: 3 },
      ],
    );
  }),
);
