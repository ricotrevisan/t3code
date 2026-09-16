import * as SqlClient from "effect/unstable/sql/SqlClient";
import * as SqlSchema from "effect/unstable/sql/SqlSchema";
import * as Effect from "effect/Effect";
import * as Layer from "effect/Layer";
import * as Option from "effect/Option";
import * as Schema from "effect/Schema";

import {
  PositiveInt,
  ProviderAdapterPackageId,
  ProviderAdapterPackageVersion,
} from "@t3tools/contracts";

import { toPersistenceSqlError } from "../Errors.ts";

import {
  ProjectionThreadSession,
  ProjectionThreadSessionRepository,
  type ProjectionThreadSessionRepositoryShape,
  DeleteProjectionThreadSessionInput,
  GetProjectionThreadSessionInput,
} from "../Services/ProjectionThreadSessions.ts";

const projectionThreadSessionDbFields = {
  threadId: ProjectionThreadSession.fields.threadId,
  status: ProjectionThreadSession.fields.status,
  providerName: ProjectionThreadSession.fields.providerName,
  providerInstanceId: ProjectionThreadSession.fields.providerInstanceId,
  runtimeMode: ProjectionThreadSession.fields.runtimeMode,
  activeTurnId: ProjectionThreadSession.fields.activeTurnId,
  lastError: ProjectionThreadSession.fields.lastError,
  updatedAt: ProjectionThreadSession.fields.updatedAt,
};

const ProjectionThreadSessionDbRow = Schema.Union([
  Schema.Struct({
    ...projectionThreadSessionDbFields,
    adapterPackageId: Schema.Null,
    adapterPackageVersion: Schema.Null,
    adapterPackageProtocolVersion: Schema.Null,
  }),
  Schema.Struct({
    ...projectionThreadSessionDbFields,
    adapterPackageId: ProviderAdapterPackageId,
    adapterPackageVersion: ProviderAdapterPackageVersion,
    adapterPackageProtocolVersion: PositiveInt,
  }),
]);

const toProjectionThreadSession = (
  row: Schema.Schema.Type<typeof ProjectionThreadSessionDbRow>,
): ProjectionThreadSession => ({
  threadId: row.threadId,
  status: row.status,
  providerName: row.providerName,
  providerInstanceId: row.providerInstanceId,
  ...(row.adapterPackageId !== null
    ? {
        adapterPackage: {
          id: row.adapterPackageId,
          version: row.adapterPackageVersion,
          protocolVersion: row.adapterPackageProtocolVersion,
        },
      }
    : {}),
  runtimeMode: row.runtimeMode,
  activeTurnId: row.activeTurnId,
  lastError: row.lastError,
  updatedAt: row.updatedAt,
});

const makeProjectionThreadSessionRepository = Effect.gen(function* () {
  const sql = yield* SqlClient.SqlClient;

  const upsertProjectionThreadSessionRow = SqlSchema.void({
    Request: ProjectionThreadSession,
    execute: (row) =>
      sql`
        INSERT INTO projection_thread_sessions (
          thread_id,
          status,
          provider_name,
          provider_instance_id,
          adapter_package_id,
          adapter_package_version,
          adapter_package_protocol_version,
          runtime_mode,
          active_turn_id,
          last_error,
          updated_at
        )
        VALUES (
          ${row.threadId},
          ${row.status},
          ${row.providerName},
          ${row.providerInstanceId},
          ${row.adapterPackage?.id ?? null},
          ${row.adapterPackage?.version ?? null},
          ${row.adapterPackage?.protocolVersion ?? null},
          ${row.runtimeMode},
          ${row.activeTurnId},
          ${row.lastError},
          ${row.updatedAt}
        )
        ON CONFLICT (thread_id)
        DO UPDATE SET
          status = excluded.status,
          provider_name = excluded.provider_name,
          provider_instance_id = excluded.provider_instance_id,
          adapter_package_id = excluded.adapter_package_id,
          adapter_package_version = excluded.adapter_package_version,
          adapter_package_protocol_version = excluded.adapter_package_protocol_version,
          runtime_mode = excluded.runtime_mode,
          active_turn_id = excluded.active_turn_id,
          last_error = excluded.last_error,
          updated_at = excluded.updated_at
      `,
  });

  const getProjectionThreadSessionRow = SqlSchema.findOneOption({
    Request: GetProjectionThreadSessionInput,
    Result: ProjectionThreadSessionDbRow,
    execute: ({ threadId }) =>
      sql`
        SELECT
          thread_id AS "threadId",
          status,
          provider_name AS "providerName",
          provider_instance_id AS "providerInstanceId",
          adapter_package_id AS "adapterPackageId",
          adapter_package_version AS "adapterPackageVersion",
          adapter_package_protocol_version AS "adapterPackageProtocolVersion",
          runtime_mode AS "runtimeMode",
          active_turn_id AS "activeTurnId",
          last_error AS "lastError",
          updated_at AS "updatedAt"
        FROM projection_thread_sessions
        WHERE thread_id = ${threadId}
      `,
  });

  const deleteProjectionThreadSessionRow = SqlSchema.void({
    Request: DeleteProjectionThreadSessionInput,
    execute: ({ threadId }) =>
      sql`
        DELETE FROM projection_thread_sessions
        WHERE thread_id = ${threadId}
      `,
  });

  const upsert: ProjectionThreadSessionRepositoryShape["upsert"] = (row) =>
    upsertProjectionThreadSessionRow(row).pipe(
      Effect.mapError(toPersistenceSqlError("ProjectionThreadSessionRepository.upsert:query")),
    );

  const getByThreadId: ProjectionThreadSessionRepositoryShape["getByThreadId"] = (input) =>
    getProjectionThreadSessionRow(input).pipe(
      Effect.mapError(
        toPersistenceSqlError("ProjectionThreadSessionRepository.getByThreadId:query"),
      ),
      Effect.map(Option.map(toProjectionThreadSession)),
    );

  const deleteByThreadId: ProjectionThreadSessionRepositoryShape["deleteByThreadId"] = (input) =>
    deleteProjectionThreadSessionRow(input).pipe(
      Effect.mapError(
        toPersistenceSqlError("ProjectionThreadSessionRepository.deleteByThreadId:query"),
      ),
    );

  return {
    upsert,
    getByThreadId,
    deleteByThreadId,
  } satisfies ProjectionThreadSessionRepositoryShape;
});

export const ProjectionThreadSessionRepositoryLive = Layer.effect(
  ProjectionThreadSessionRepository,
  makeProjectionThreadSessionRepository,
);
