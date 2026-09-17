import * as NodeServices from "@effect/platform-node/NodeServices";
import { assert, describe, it } from "@effect/vitest";
import * as Effect from "effect/Effect";
import * as Layer from "effect/Layer";

import { ServerConfig } from "../config.ts";
import { runPiConformance } from "./testFixtures/piConformanceRunner.mjs";

const testLayer = ServerConfig.layerTest(process.cwd(), {
  prefix: "pi-adapter-conformance-test-",
}).pipe(Layer.provideMerge(NodeServices.layer));

describe("pi rpc adapter conformance", () => {
  it.live("drives Pi RPC through the shipped provider package", () =>
    Effect.gen(function* () {
      const result = yield* runPiConformance;
      const failedChecks = result.checks.filter((entry) => !entry.ok);
      const failureDetail = failedChecks
        .map((entry) => `${entry.name}: ${entry.detail ?? "failed"}`)
        .join("\n");

      assert.equal(failedChecks.length, 0, failureDetail);
      assert.ok(result.checks.length > 0, "The scenario must execute checks.");
      assert.ok(result.eventCount > 0, "The scenario must consume canonical events.");
    }).pipe(Effect.provide(testLayer)),
  );
});
