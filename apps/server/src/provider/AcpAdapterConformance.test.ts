import * as NodeServices from "@effect/platform-node/NodeServices";
import { assert, describe, it } from "@effect/vitest";
import * as Effect from "effect/Effect";
import * as Layer from "effect/Layer";
import { HttpClient, HttpClientResponse } from "effect/unstable/http";

import { ServerConfig } from "../config.ts";
import { ServerSettingsService } from "../serverSettings.ts";
import { runAcpConformance } from "./testFixtures/acpConformanceRunner.mjs";

const TestHttpClientLive = Layer.succeed(
  HttpClient.HttpClient,
  HttpClient.make((request) =>
    Effect.succeed(HttpClientResponse.fromWeb(request, Response.json({ version: "1.0.0" }))),
  ),
);

const testLayer = ServerConfig.layerTest(process.cwd(), {
  prefix: "acp-adapter-conformance-test-",
}).pipe(
  Layer.provideMerge(NodeServices.layer),
  Layer.provideMerge(ServerSettingsService.layerTest()),
  Layer.provideMerge(TestHttpClientLive),
);

describe("generic ACP stdio adapter conformance", () => {
  it.live("drives ACP JSON-RPC through the external provider bridge", () =>
    Effect.gen(function* () {
      const result = yield* runAcpConformance;
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
