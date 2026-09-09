import * as NodeServices from "@effect/platform-node/NodeServices";
import { expect, it } from "@effect/vitest";
import { EnvironmentId, ProviderInstanceId, ThreadId } from "@t3tools/contracts";
import * as Effect from "effect/Effect";
import * as Result from "effect/Result";
import * as Stream from "effect/Stream";

import * as PreviewAutomationBroker from "../mcp/PreviewAutomationBroker.ts";
import {
  LAB_CHROMIUM_CLIENT_ID,
  executeLabPreviewOperation,
  isLabChromiumAutomationEnabled,
  makeLabPreviewAutomationHost,
  resolveLabNavigationUrl,
  serializeLabHostError,
  spawnLabBrowser,
} from "./LabChromiumAutomationHost.ts";

const environmentId = EnvironmentId.make("environment-1");
const scope = {
  environmentId,
  threadId: ThreadId.make("thread-1"),
  providerSessionId: "provider-session-1",
  providerInstanceId: ProviderInstanceId.make("codex"),
  capabilities: new Set(["preview"] as const),
  issuedAt: 1,
};

const makeBroker = PreviewAutomationBroker.make.pipe(Effect.provide(NodeServices.layer));

it("resolves public hosts over https and loopback ports over http", () => {
  expect(resolveLabNavigationUrl({ url: "example.com" })).toBe("https://example.com/");
  expect(
    resolveLabNavigationUrl({
      target: { kind: "environment-port", port: 5173, path: "/settings" },
    }),
  ).toBe("http://127.0.0.1:5173/settings");
});

it("serializes unknown failures as execution errors", () => {
  const serialized = serializeLabHostError(new Error("boom"), {
    requestId: "preview-1",
    threadId: scope.threadId,
    operation: "status",
    input: {},
    timeoutMs: 15_000,
  });
  expect(serialized._tag).toBe("PreviewAutomationExecutionError");
});

it.each([
  [{ mode: "web", startupPresentation: "headless" }, true],
  [{ mode: "web", startupPresentation: "browser" }, false],
  [{ mode: "desktop", startupPresentation: "headless" }, false],
  [{ mode: "desktop", startupPresentation: "browser" }, false],
] as const)("enables the lab Chromium host only for headless web startup", (config, expected) => {
  expect(isLabChromiumAutomationEnabled(config)).toBe(expected);
});

it("reports a missing browser command without an unhandled child-process error", async () => {
  await expect(
    spawnLabBrowser(`t3-browser-command-that-does-not-exist-${process.pid}`),
  ).rejects.toMatchObject({ code: "ENOENT" });
});

it.effect("lab host identity lets the broker route preview open and status", () =>
  Effect.scoped(
    Effect.gen(function* () {
      const broker = yield* makeBroker;
      const host = makeLabPreviewAutomationHost(environmentId);
      expect(host.clientId).toBe(LAB_CHROMIUM_CLIENT_ID);
      const events = yield* broker.connect(host);
      const requests = events.pipe(
        Stream.filterMap((event) => {
          if (event.type === "connected") return Result.failVoid;
          return Result.succeed(event);
        }),
      );
      yield* Stream.runForEach(requests, (event) =>
        broker.respond({
          clientId: LAB_CHROMIUM_CLIENT_ID,
          connectionId: event.connectionId,
          requestId: event.request.requestId,
          ok: true,
          result: {
            available: true,
            visible: false,
            tabId: "tab_lab",
            url: "about:blank",
            title: "",
            loading: false,
          },
        }),
      ).pipe(Effect.forkScoped);
      yield* Effect.yieldNow;

      const opened = yield* broker.invoke<{ available: boolean; tabId: string }>({
        scope,
        operation: "open",
        input: {},
      });
      expect(opened.available).toBe(true);
      expect(opened.tabId).toBe("tab_lab");

      const status = yield* broker.invoke<{ available: boolean }>({
        scope,
        operation: "status",
        input: {},
      });
      expect(status.available).toBe(true);
    }),
  ),
);

it("exports the live Chromium executor", () => {
  expect(typeof executeLabPreviewOperation).toBe("function");
});
