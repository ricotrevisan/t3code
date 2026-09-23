/**
 * End-to-end conformance for the echo adapter package.
 *
 * Loads a real external package module through the trusted-local loader,
 * drives a real harness subprocess through the V1 host process supervisor,
 * and checks the canonical translation the bridge performs.
 */
import * as NodeServices from "@effect/platform-node/NodeServices";
import { assert, describe, it } from "@effect/vitest";
import {
  ProviderAdapterPackageId,
  ProviderAdapterPackageVersion,
  ProviderDriverKind,
  ProviderInstanceId,
  ThreadId,
} from "@t3tools/contracts";
import * as Effect from "effect/Effect";
import * as FileSystem from "effect/FileSystem";
import * as Fiber from "effect/Fiber";
import * as Layer from "effect/Layer";
import { HttpClient, HttpClientResponse } from "effect/unstable/http";
import * as Schema from "effect/Schema";
import * as Stream from "effect/Stream";

import { ServerConfig } from "../config.ts";
import { ServerSettingsService } from "../serverSettings.ts";
import { makeProviderInstanceRegistry } from "./Layers/ProviderInstanceRegistryLive.ts";
import { loadTrustedProviderAdapterPackages } from "./TrustedLocalProviderAdapters.ts";

const ECHO_DRIVER = ProviderDriverKind.make("echoHarness");
const ECHO_INSTANCE = ProviderInstanceId.make("echo_conformance");
const ECHO_THREAD = ThreadId.make("echo-conformance-thread");
const ECHO_PACKAGE = {
  id: ProviderAdapterPackageId.make("echo-conformance"),
  version: ProviderAdapterPackageVersion.make("1.0.0"),
  protocolVersion: 1,
};

const encodeUnknownJson = Schema.encodeSync(Schema.fromJsonString(Schema.Unknown));

const writeRegistry = Effect.fn("writeRegistry")(function* (
  packages: ReadonlyArray<Record<string, unknown>>,
) {
  const fileSystem = yield* FileSystem.FileSystem;
  const directory = yield* fileSystem.makeTempDirectoryScoped({ prefix: "t3-echo-conformance-" });
  const registryPath = `${directory}/provider-adapters.json`;
  yield* fileSystem.writeFileString(
    registryPath,
    encodeUnknownJson({ schemaVersion: 1, packages }),
  );
  return registryPath;
});

const makeEchoInstance = () =>
  Effect.gen(function* () {
    const registryPath = yield* writeRegistry([
      {
        id: "echo-conformance",
        version: "1.0.0",
        driver: ECHO_DRIVER,
        modulePath: new URL("./testFixtures/echoAdapterPackage.mjs", import.meta.url).pathname,
        enabled: true,
      },
    ]);
    const loaded = yield* loadTrustedProviderAdapterPackages({
      registryPath,
      reservedDriverKinds: new Set(),
    });
    assert.lengthOf(loaded.drivers, 1);
    assert.deepStrictEqual(loaded.diagnostics, []);
    const { registry } = yield* makeProviderInstanceRegistry({
      drivers: loaded.drivers,
      configMap: {
        [ECHO_INSTANCE]: {
          driver: ECHO_DRIVER,
          adapterPackage: ECHO_PACKAGE,
          displayName: "Echo Conformance",
          config: {},
        },
      },
    });
    const instance = yield* registry.getInstance(ECHO_INSTANCE);
    assert.isDefined(instance);
    return instance;
  });

const TestHttpClientLive = Layer.succeed(
  HttpClient.HttpClient,
  HttpClient.make((request) =>
    Effect.succeed(HttpClientResponse.fromWeb(request, Response.json({ version: "1.0.0" }))),
  ),
);

const testLayer = ServerConfig.layerTest(process.cwd(), {
  prefix: "echo-adapter-conformance-test-",
}).pipe(
  Layer.provideMerge(NodeServices.layer),
  Layer.provideMerge(ServerSettingsService.layerTest()),
  Layer.provideMerge(TestHttpClientLive),
);

describe("echo adapter conformance", () => {
  it.live("drives a harness subprocess through the V1 seam", () =>
    Effect.gen(function* () {
      const instance = yield* makeEchoInstance();

      // Plan-mode turns stay open in the echo harness until interrupted.
      const sendTurn = (input: string, interactionMode?: "plan") =>
        instance.adapter.sendTurn(
          interactionMode === undefined
            ? { threadId: ECHO_THREAD, input, attachments: [] }
            : { threadId: ECHO_THREAD, input, attachments: [], interactionMode },
        );

      const collected = yield* instance.adapter.streamEvents.pipe(
        Stream.take(6),
        Stream.runCollect,
        Effect.forkChild,
      );

      const session = yield* instance.adapter.startSession({
        threadId: ECHO_THREAD,
        providerInstanceId: ECHO_INSTANCE,
        runtimeMode: "full-access",
      });
      assert.equal(session.status, "ready");
      assert.equal(session.provider, ECHO_DRIVER);

      const firstTurn = yield* sendTurn("hello echo");
      assert.isDefined(firstTurn.turnId);
      assert.deepStrictEqual(firstTurn.resumeCursor, { echo: ECHO_THREAD });

      const heldTurn = yield* sendTurn("interrupt me", "plan");
      assert.isDefined(heldTurn.turnId);
      yield* instance.adapter.interruptTurn(ECHO_THREAD);

      const events = Array.from(yield* Fiber.join(collected));
      assert.deepStrictEqual(
        events.map((event) => event.type),
        [
          "session.started",
          "turn.started",
          "content.delta",
          "turn.completed",
          "turn.started",
          "turn.aborted",
        ],
      );
      assert.equal(events[0]?.threadId, ECHO_THREAD);
      assert.equal(events[1]?.turnId, firstTurn.turnId);
      assert.equal(events[2]?.turnId, firstTurn.turnId);
      if (events[2]?.type === "content.delta") {
        assert.equal(events[2].payload.streamKind, "assistant_text");
        assert.equal(events[2].payload.delta, "ohce olleh");
      }
      if (events[3]?.type === "turn.completed") {
        assert.equal(events[3].payload.state, "completed");
      }
      assert.equal(events[4]?.turnId, heldTurn.turnId);
      if (events[5]?.type === "turn.aborted") {
        assert.equal(events[5].payload.reason, "interrupted by echo harness");
      }
      for (const event of events) {
        assert.equal(event.providerInstanceId, ECHO_INSTANCE);
      }

      assert.isTrue(yield* instance.adapter.hasSession(ECHO_THREAD));
      const sessions = yield* instance.adapter.listSessions();
      assert.lengthOf(sessions, 1);
      assert.equal(sessions[0]?.activeTurnId, undefined);

      const thread = yield* instance.adapter.readThread(ECHO_THREAD);
      assert.lengthOf(thread.turns, 2);
      assert.equal(thread.turns[0]?.id, firstTurn.turnId);
      assert.equal(thread.turns[1]?.id, heldTurn.turnId);
      const heldItems = thread.turns[1]?.items ?? [];
      assert.isTrue((heldItems[0] as { interrupted?: boolean }).interrupted);

      const rolledBack = yield* instance.adapter.rollbackThread(ECHO_THREAD, 1);
      assert.lengthOf(rolledBack.turns, 1);
      assert.equal(rolledBack.turns[0]?.id, firstTurn.turnId);

      const snapshot = yield* instance.snapshot.getSnapshot;
      assert.equal(snapshot.driver, ECHO_DRIVER);
      assert.equal(snapshot.instanceId, ECHO_INSTANCE);
      assert.deepStrictEqual(snapshot.adapterCapabilities, {
        protocolVersion: 1,
        features: ["turn.interrupt"],
      });
      assert.deepStrictEqual(snapshot.adapterPackage, ECHO_PACKAGE);
    }).pipe(Effect.provide(testLayer), Effect.scoped),
  );

  it.live("stops the harness gracefully without a crash event", () =>
    Effect.gen(function* () {
      const instance = yield* makeEchoInstance();
      const exitEvent = yield* instance.adapter.streamEvents.pipe(
        Stream.take(2),
        Stream.runCollect,
        Effect.forkChild,
      );

      yield* instance.adapter.startSession({
        threadId: ECHO_THREAD,
        providerInstanceId: ECHO_INSTANCE,
        runtimeMode: "full-access",
      });
      yield* instance.adapter.stopSession(ECHO_THREAD);

      assert.isFalse(yield* instance.adapter.hasSession(ECHO_THREAD));
      assert.deepStrictEqual(yield* instance.adapter.listSessions(), []);

      // session.started from start, then the graceful session.exited. No
      // host-generated runtime.error may appear for an intentional stop.
      const events = Array.from(yield* Fiber.join(exitEvent));
      assert.equal(events[0]?.type, "session.started");
      assert.equal(events[1]?.type, "session.exited");
      if (events[1]?.type === "session.exited") {
        assert.equal(events[1].payload.exitKind, "graceful");
      }
    }).pipe(Effect.provide(testLayer), Effect.scoped),
  );
});
