import * as NodeURL from "node:url";

import * as NodeServices from "@effect/platform-node/NodeServices";
import { assert, describe, it } from "@effect/vitest";
import * as Effect from "effect/Effect";
import * as Schema from "effect/Schema";
import { ChildProcess, ChildProcessSpawner } from "effect/unstable/process";

import { collectUint8StreamText } from "../stream/collectUint8StreamText.ts";

const runnerPath = NodeURL.fileURLToPath(
  new URL("./testFixtures/acpConformanceRunner.mjs", import.meta.url),
);
const RunnerResult = Schema.Struct({
  ok: Schema.Boolean,
  eventCount: Schema.optional(Schema.Number),
  error: Schema.optional(Schema.String),
  checks: Schema.Array(
    Schema.Struct({
      name: Schema.String,
      ok: Schema.Boolean,
      detail: Schema.optional(Schema.String),
    }),
  ),
});
const decodeRunnerResult = Schema.decodeUnknownEffect(Schema.fromJsonString(RunnerResult));

describe("generic ACP stdio adapter conformance", () => {
  it.live("drives ACP JSON-RPC through the external provider bridge", () =>
    Effect.gen(function* () {
      const spawner = yield* ChildProcessSpawner.ChildProcessSpawner;
      const child = yield* spawner.spawn(ChildProcess.make(process.execPath, [runnerPath]));
      const [stdout, stderr, exitCode] = yield* Effect.all(
        [
          collectUint8StreamText({ stream: child.stdout, maxBytes: 1024 * 1024 }),
          collectUint8StreamText({ stream: child.stderr, maxBytes: 1024 * 1024 }),
          child.exitCode,
        ],
        { concurrency: "unbounded" },
      );
      const result = yield* decodeRunnerResult(stdout.text.trim());
      const failedChecks = result.checks.filter((entry) => !entry.ok);
      const failureDetail = [
        result.error,
        ...failedChecks.map((entry) => `${entry.name}: ${entry.detail ?? "failed"}`),
      ]
        .filter((entry) => entry !== undefined)
        .join("\n");

      assert.equal(Number(exitCode), 0, stderr.text);
      assert.equal(stderr.text, "");
      assert.equal(result.ok, true, failureDetail);
      assert.ok(result.checks.length > 0, "The scenario must execute checks.");
      assert.ok((result.eventCount ?? 0) > 0, "The scenario must consume canonical events.");
    }).pipe(Effect.provide(NodeServices.layer), Effect.scoped),
  );
});
