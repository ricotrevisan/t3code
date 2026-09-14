import * as NodeServices from "@effect/platform-node/NodeServices";
import { assert, describe, it } from "@effect/vitest";
import {
  ProviderAdapterManifestV1,
  ProviderDriverKind,
  ProviderInstanceId,
  ServerProvider,
  ThreadId,
  type ProviderInstanceConfigMap,
} from "@t3tools/contracts";
import * as Effect from "effect/Effect";
import * as FileSystem from "effect/FileSystem";
import * as Path from "effect/Path";
import * as Schema from "effect/Schema";
import * as Stream from "effect/Stream";

import type { ProviderAdapterShape } from "./Services/ProviderAdapter.ts";
import type { AnyProviderDriver, ProviderInstance } from "./ProviderDriver.ts";
import { makeProviderInstanceRegistry } from "./Layers/ProviderInstanceRegistryLive.ts";
import {
  type ExternalProviderAdapterPackageV1,
  loadTrustedProviderAdapterPackages,
} from "./TrustedLocalProviderAdapters.ts";

const decodeServerProvider = Schema.decodeUnknownSync(ServerProvider);

const ECHO_DRIVER = ProviderDriverKind.make("echoHarness");
const ECHO_INSTANCE = ProviderInstanceId.make("echo_local");
const NOW = "2026-09-14T00:00:00.000Z";

const decodeManifest = Schema.decodeUnknownSync(ProviderAdapterManifestV1);
const manifest = (overrides: Record<string, unknown> = {}) =>
  decodeManifest({
    protocolVersion: 1,
    id: "echo-adapter",
    version: "1.0.0",
    driver: ECHO_DRIVER,
    displayName: "Echo Adapter",
    hostProtocol: { minimum: 1, maximum: 1 },
    transport: {
      kind: "supervised-stdio",
      protocol: "jsonl-rpc",
      sessionConcurrency: "one-per-process",
    },
    capabilities: ["turn.interrupt", "stream.reasoning"],
    configSchema: { type: "object", additionalProperties: false },
    ...overrides,
  });

const makeEchoDriver = (
  negotiatedFeatures: ProviderAdapterManifestV1["capabilities"] = [
    "turn.interrupt",
    "stream.reasoning",
  ],
): AnyProviderDriver<never> => ({
  driverKind: ECHO_DRIVER,
  metadata: { displayName: "Echo Adapter" },
  configSchema: Schema.Struct({}),
  defaultConfig: () => ({}),
  create: ({ instanceId, displayName, enabled }) => {
    const snapshot = decodeServerProvider({
      instanceId,
      driver: ECHO_DRIVER,
      displayName,
      enabled,
      installed: true,
      version: "1.0.0",
      status: enabled ? "ready" : "disabled",
      auth: { status: "unknown" },
      checkedAt: NOW,
      availability: "available",
      models: [],
      slashCommands: [],
      skills: [],
    });
    const unsupported = () => Effect.die("not exercised by package-loader tests");
    const adapter: ProviderAdapterShape<never> = {
      provider: ECHO_DRIVER,
      capabilities: {
        sessionModelSwitch: "unsupported",
        protocol: { protocolVersion: 1, features: negotiatedFeatures },
      },
      startSession: (input) =>
        Effect.succeed({
          provider: ECHO_DRIVER,
          providerInstanceId: input.providerInstanceId,
          status: "ready",
          runtimeMode: input.runtimeMode,
          threadId: input.threadId,
          createdAt: NOW,
          updatedAt: NOW,
        }),
      sendTurn: unsupported,
      interruptTurn: () => Effect.void,
      respondToRequest: unsupported,
      respondToUserInput: unsupported,
      stopSession: () => Effect.void,
      listSessions: () => Effect.succeed([]),
      hasSession: () => Effect.succeed(false),
      readThread: unsupported,
      rollbackThread: unsupported,
      stopAll: () => Effect.void,
      streamEvents: Stream.empty,
    };
    const textGeneration = {
      generateCommitMessage: unsupported,
      generatePrContent: unsupported,
      generateBranchName: unsupported,
      generateThreadTitle: unsupported,
    };
    return Effect.succeed({
      instanceId,
      driverKind: ECHO_DRIVER,
      continuationIdentity: {
        driverKind: ECHO_DRIVER,
        continuationKey: `echo:instance:${instanceId}`,
      },
      displayName,
      enabled,
      snapshot: {
        maintenanceCapabilities: {
          provider: ECHO_DRIVER,
          packageName: null,
          update: null,
        },
        getSnapshot: Effect.succeed(snapshot),
        refresh: Effect.succeed(snapshot),
        streamChanges: Stream.empty,
      },
      adapter,
      textGeneration,
    } satisfies ProviderInstance);
  },
});

const encodeUnknownJson = Schema.encodeSync(Schema.fromJsonString(Schema.Unknown));

const writeRegistry = Effect.fn("writeRegistry")(function* (
  packages: ReadonlyArray<Record<string, unknown>>,
) {
  const fileSystem = yield* FileSystem.FileSystem;
  const directory = yield* fileSystem.makeTempDirectoryScoped({ prefix: "t3-adapter-registry-" });
  const registryPath = `${directory}/provider-adapters.json`;
  yield* fileSystem.writeFileString(
    registryPath,
    encodeUnknownJson({ schemaVersion: 1, packages }),
  );
  return registryPath;
});

const registration = {
  id: "echo-adapter",
  version: "1.0.0",
  driver: ECHO_DRIVER,
  modulePath: "/trusted/echo-adapter.mjs",
  enabled: true,
};

const registryConfig: ProviderInstanceConfigMap = {
  [ECHO_INSTANCE]: {
    driver: ECHO_DRIVER,
    displayName: "Echo Local",
    config: {},
  },
};

const testLayer = NodeServices.layer;

describe("trusted local provider adapter packages", () => {
  it.effect("loads a registered package and materializes its provider instance", () =>
    Effect.gen(function* () {
      const registryPath = yield* writeRegistry([registration]);
      const adapterPackage: ExternalProviderAdapterPackageV1 = {
        manifest: manifest(),
        driver: makeEchoDriver(),
      };
      const loaded = yield* loadTrustedProviderAdapterPackages<never>({
        registryPath,
        reservedDriverKinds: new Set(),
        loadModule: async () => ({ default: adapterPackage }),
      });

      assert.lengthOf(loaded.drivers, 1);
      assert.deepStrictEqual(loaded.diagnostics, []);

      const secondInstanceId = ProviderInstanceId.make("echo_work");
      const { registry } = yield* makeProviderInstanceRegistry({
        drivers: loaded.drivers,
        unavailableDriverReasons: loaded.unavailableDriverReasons,
        configMap: {
          ...registryConfig,
          [secondInstanceId]: {
            driver: ECHO_DRIVER,
            displayName: "Echo Work",
            config: {},
          },
        },
      });
      assert.lengthOf(yield* registry.listInstances, 2);
      const secondInstance = yield* registry.getInstance(secondInstanceId);
      assert.deepStrictEqual(secondInstance?.adapterPackage, {
        id: "echo-adapter",
        version: "1.0.0",
        protocolVersion: 1,
      });
      const instance = yield* registry.getInstance(ECHO_INSTANCE);
      assert.isDefined(instance);
      assert.deepStrictEqual(instance.adapterPackage, {
        id: "echo-adapter",
        version: "1.0.0",
        protocolVersion: 1,
      });
      const snapshot = yield* instance.snapshot.getSnapshot;
      assert.deepStrictEqual(snapshot.adapterPackage, instance.adapterPackage);
      assert.equal(snapshot.instanceId, ECHO_INSTANCE);
      const session = yield* instance.adapter.startSession({
        threadId: ThreadId.make("echo-thread"),
        providerInstanceId: ECHO_INSTANCE,
        runtimeMode: "full-access",
      });
      assert.deepStrictEqual(session.adapterPackage, instance.adapterPackage);
    }).pipe(Effect.provide(testLayer), Effect.scoped),
  );

  it.effect("fails an incompatible package closed and keeps its instance diagnosable", () =>
    Effect.gen(function* () {
      const registryPath = yield* writeRegistry([registration]);
      const loaded = yield* loadTrustedProviderAdapterPackages<never>({
        registryPath,
        reservedDriverKinds: new Set(),
        loadModule: async () => ({
          default: {
            manifest: manifest({ hostProtocol: { minimum: 2, maximum: 2 } }),
            driver: makeEchoDriver(),
          },
        }),
      });

      assert.deepStrictEqual(loaded.drivers, []);
      assert.equal(loaded.diagnostics[0]?.code, "package-incompatible");

      const { registry } = yield* makeProviderInstanceRegistry({
        drivers: loaded.drivers,
        unavailableDriverReasons: loaded.unavailableDriverReasons,
        configMap: registryConfig,
      });
      assert.deepStrictEqual(yield* registry.listInstances, []);
      const unavailable = yield* registry.listUnavailable;
      assert.lengthOf(unavailable, 1);
      assert.equal(unavailable[0]?.availability, "unavailable");
      assert.include(unavailable[0]?.unavailableReason ?? "", "incompatible");
    }).pipe(Effect.provide(testLayer), Effect.scoped),
  );

  it.effect("rejects negotiated capabilities the package did not declare", () =>
    Effect.gen(function* () {
      const registryPath = yield* writeRegistry([registration]);
      const loaded = yield* loadTrustedProviderAdapterPackages<never>({
        registryPath,
        reservedDriverKinds: new Set(),
        loadModule: async () => ({
          default: {
            manifest: manifest({ capabilities: ["turn.interrupt"] }),
            driver: makeEchoDriver(["turn.interrupt", "stream.reasoning"]),
          },
        }),
      });
      const { registry } = yield* makeProviderInstanceRegistry({
        drivers: loaded.drivers,
        configMap: registryConfig,
      });

      assert.deepStrictEqual(yield* registry.listInstances, []);
      const unavailable = yield* registry.listUnavailable;
      assert.include(unavailable[0]?.unavailableReason ?? "", "undeclared capabilities");
    }).pipe(Effect.provide(testLayer), Effect.scoped),
  );

  it.effect("imports a registered module from the local filesystem", () =>
    Effect.gen(function* () {
      const path = yield* Path.Path;
      const modulePath = yield* path.fromFileUrl(
        new URL("./testFixtures/externalEchoAdapter.mjs", import.meta.url),
      );
      const registryPath = yield* writeRegistry([{ ...registration, modulePath }]);
      const loaded = yield* loadTrustedProviderAdapterPackages<never>({
        registryPath,
        reservedDriverKinds: new Set(),
      });

      assert.lengthOf(loaded.drivers, 1);
      assert.deepStrictEqual(loaded.diagnostics, []);
    }).pipe(Effect.provide(testLayer), Effect.scoped),
  );

  it.effect("does not allow an external package to replace a built-in driver", () =>
    Effect.gen(function* () {
      const registryPath = yield* writeRegistry([registration]);
      const loaded = yield* loadTrustedProviderAdapterPackages<never>({
        registryPath,
        reservedDriverKinds: new Set([ECHO_DRIVER]),
        loadModule: async () => ({
          default: { manifest: manifest(), driver: makeEchoDriver() },
        }),
      });

      assert.deepStrictEqual(loaded.drivers, []);
      assert.equal(loaded.diagnostics[0]?.code, "driver-conflict");
    }).pipe(Effect.provide(testLayer), Effect.scoped),
  );
});
