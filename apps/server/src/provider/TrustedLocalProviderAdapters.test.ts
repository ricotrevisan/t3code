import * as NodeServices from "@effect/platform-node/NodeServices";
import { assert, describe, it } from "@effect/vitest";
import {
  ProviderAdapterManifestV1,
  ProviderAdapterPackageId,
  ProviderAdapterPackageVersion,
  ProviderDriverKind,
  ProviderInstanceId,
  ServerProvider,
  ThreadId,
  type ProviderInstanceConfigMap,
} from "@t3tools/contracts";
import * as Effect from "effect/Effect";
import * as FileSystem from "effect/FileSystem";
import * as Layer from "effect/Layer";
import * as Path from "effect/Path";
import * as Schema from "effect/Schema";
import * as Stream from "effect/Stream";
import {
  defineProviderAdapterV1,
  type ProviderAdapterHost,
  type ProviderAdapterHostV1,
  type ProviderAdapterHostV2,
  type ProviderAdapterPackageV1,
} from "@t3tools/provider-adapter";

import { ServerConfig } from "../config.ts";
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

const makeEchoPackage = <Host extends ProviderAdapterHost = ProviderAdapterHostV1>(
  negotiatedFeatures: ProviderAdapterManifestV1["capabilities"] = [
    "turn.interrupt",
    "stream.reasoning",
  ],
  manifestOverrides: Record<string, unknown> = {},
  inspectHost?: ((host: Host) => void) | undefined,
): ProviderAdapterPackageV1<Record<string, never>, Host> =>
  defineProviderAdapterV1<Record<string, never>, Host>({
    manifest: manifest(manifestOverrides),
    configSchema: Schema.Struct({}),
    defaultConfig: () => ({}),
    create: ({ instanceId, displayName, enabled }, host) => {
      inspectHost?.(host);
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
      return Effect.succeed({
        continuationKey: `echo:instance:${instanceId}`,
        snapshot: {
          getSnapshot: Effect.succeed(snapshot),
          refresh: Effect.succeed(snapshot),
          streamChanges: Stream.empty,
        },
        adapter: {
          capabilities: { protocolVersion: 1, features: negotiatedFeatures },
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
        },
      });
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

const ECHO_PACKAGE_REFERENCE = {
  id: ProviderAdapterPackageId.make("echo-adapter"),
  version: ProviderAdapterPackageVersion.make("1.0.0"),
  protocolVersion: 1,
};

const registryConfig: ProviderInstanceConfigMap = {
  [ECHO_INSTANCE]: {
    driver: ECHO_DRIVER,
    adapterPackage: ECHO_PACKAGE_REFERENCE,
    displayName: "Echo Local",
    config: {},
  },
};

const testLayer = ServerConfig.layerTest(process.cwd(), {
  prefix: "trusted-local-provider-adapters-test-",
}).pipe(Layer.provideMerge(NodeServices.layer));

describe("trusted local provider adapter packages", () => {
  it.effect("loads a registered package and materializes its provider instance", () =>
    Effect.gen(function* () {
      const registryPath = yield* writeRegistry([registration]);
      const adapterPackage: ExternalProviderAdapterPackageV1<Record<string, never>> =
        makeEchoPackage();
      const loaded = yield* loadTrustedProviderAdapterPackages({
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
            adapterPackage: ECHO_PACKAGE_REFERENCE,
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
      assert.deepStrictEqual(snapshot.adapterConfigSchema, {
        type: "object",
        additionalProperties: false,
      });
      assert.deepStrictEqual(snapshot.adapterCapabilities, {
        protocolVersion: 1,
        features: ["turn.interrupt", "stream.reasoning"],
      });
      assert.equal(snapshot.instanceId, ECHO_INSTANCE);
      const session = yield* instance.adapter.startSession({
        threadId: ThreadId.make("echo-thread"),
        providerInstanceId: ECHO_INSTANCE,
        runtimeMode: "full-access",
      });
      assert.deepStrictEqual(session.adapterPackage, instance.adapterPackage);
      const failedTurn = yield* instance.adapter
        .sendTurn({ threadId: session.threadId, input: "hello", attachments: [] })
        .pipe(Effect.result);
      assert.equal(failedTurn._tag, "Failure");
      if (
        failedTurn._tag === "Failure" &&
        failedTurn.failure._tag === "ProviderAdapterRequestError"
      ) {
        assert.equal(failedTurn.failure.method, "sendTurn");
      }
    }).pipe(Effect.provide(testLayer), Effect.scoped),
  );

  it.effect("negotiates the highest supported host protocol and preserves the V1 shape", () =>
    Effect.gen(function* () {
      const v1Hosts: Array<ProviderAdapterHostV1> = [];
      const registryPath = yield* writeRegistry([registration]);
      const v1Package = makeEchoPackage(undefined, {}, (host) => v1Hosts.push(host));
      const loadedV1 = yield* loadTrustedProviderAdapterPackages({
        registryPath,
        reservedDriverKinds: new Set(),
        loadModule: async () => ({ default: v1Package }),
      });
      yield* makeProviderInstanceRegistry({
        drivers: loadedV1.drivers,
        configMap: registryConfig,
      });

      assert.lengthOf(v1Hosts, 1);
      assert.equal(v1Hosts[0]?.protocolVersion, 1);
      assert.deepStrictEqual(Object.keys(v1Hosts[0] ?? {}).sort(), [
        "processes",
        "protocolVersion",
      ]);

      const v2Hosts: Array<ProviderAdapterHostV2> = [];
      const v2Package: ProviderAdapterPackageV1<Record<string, never>, ProviderAdapterHostV2> = {
        ...makeEchoPackage<ProviderAdapterHostV2>(
          undefined,
          { hostProtocol: { minimum: 1, maximum: 2 } },
          (host) => v2Hosts.push(host),
        ),
        storageKey: "echo-v2-storage",
      };
      const loadedV2 = yield* loadTrustedProviderAdapterPackages({
        registryPath,
        reservedDriverKinds: new Set(),
        loadModule: async () => ({ default: v2Package }),
      });
      yield* makeProviderInstanceRegistry({
        drivers: loadedV2.drivers,
        configMap: registryConfig,
      });

      const v2Host = v2Hosts[0];
      assert.isDefined(v2Host);
      assert.equal(v2Host.protocolVersion, 2);
      assert.deepStrictEqual(Object.keys(v2Host).sort(), [
        "attachments",
        "processes",
        "protocolVersion",
        "storage",
        "workspaces",
      ]);
      assert.equal(yield* v2Host.workspaces.resolveCwd(), process.cwd());
      const sessionStorage = yield* v2Host.storage.prepareSession(ThreadId.make("host-v2-thread"));
      assert.include(sessionStorage.sharedDirectory, "echo-v2-storage");
      assert.include(sessionStorage.sessionDirectory, "host-v2-thread");
      assert.isTrue(
        yield* FileSystem.FileSystem.pipe(
          Effect.flatMap((fs) => fs.exists(sessionStorage.sessionDirectory)),
        ),
      );
    }).pipe(Effect.provide(testLayer), Effect.scoped),
  );

  it.effect("requires configured instances to pin the loaded adapter package", () =>
    Effect.gen(function* () {
      const registryPath = yield* writeRegistry([registration]);
      const loaded = yield* loadTrustedProviderAdapterPackages({
        registryPath,
        reservedDriverKinds: new Set(),
        loadModule: async () => ({
          default: makeEchoPackage(),
        }),
      });
      const unpinnedId = ProviderInstanceId.make("echo_unpinned");
      const mismatchedId = ProviderInstanceId.make("echo_old_version");
      const { registry } = yield* makeProviderInstanceRegistry({
        drivers: loaded.drivers,
        configMap: {
          [unpinnedId]: { driver: ECHO_DRIVER, config: {} },
          [mismatchedId]: {
            driver: ECHO_DRIVER,
            adapterPackage: {
              ...ECHO_PACKAGE_REFERENCE,
              version: ProviderAdapterPackageVersion.make("0.9.0"),
            },
            config: {},
          },
        },
      });

      assert.deepStrictEqual(yield* registry.listInstances, []);
      const unavailable = yield* registry.listUnavailable;
      assert.lengthOf(unavailable, 2);
      assert.include(
        unavailable.find((snapshot) => snapshot.instanceId === unpinnedId)?.unavailableReason ?? "",
        "requires an explicit adapter package reference",
      );
      const mismatched = unavailable.find((snapshot) => snapshot.instanceId === mismatchedId);
      assert.include(mismatched?.unavailableReason ?? "", "does not match loaded package");
      assert.equal(mismatched?.adapterPackage?.version, "0.9.0");
    }).pipe(Effect.provide(testLayer), Effect.scoped),
  );

  it.effect("rejects a package whose default configuration throws", () =>
    Effect.gen(function* () {
      const registryPath = yield* writeRegistry([registration]);
      const validPackage = makeEchoPackage();
      const brokenPackage = {
        ...validPackage,
        defaultConfig: () => {
          throw new Error("broken defaults");
        },
      };
      const loaded = yield* loadTrustedProviderAdapterPackages({
        registryPath,
        reservedDriverKinds: new Set(),
        loadModule: async () => ({ default: brokenPackage }),
      });

      assert.deepStrictEqual(loaded.drivers, []);
      assert.equal(loaded.diagnostics[0]?.code, "package-invalid");
      assert.include(loaded.diagnostics[0]?.detail ?? "", "broken defaults");
    }).pipe(Effect.provide(testLayer), Effect.scoped),
  );

  it.effect("turns a malformed runtime instance into an unavailable provider", () =>
    Effect.gen(function* () {
      const registryPath = yield* writeRegistry([registration]);
      const validPackage = makeEchoPackage();
      const malformedPackage = {
        ...validPackage,
        create: () => Effect.succeed({}),
      } as unknown as ExternalProviderAdapterPackageV1<Record<string, never>>;
      const loaded = yield* loadTrustedProviderAdapterPackages({
        registryPath,
        reservedDriverKinds: new Set(),
        loadModule: async () => ({ default: malformedPackage }),
      });
      const { registry } = yield* makeProviderInstanceRegistry({
        drivers: loaded.drivers,
        configMap: registryConfig,
      });

      assert.deepStrictEqual(yield* registry.listInstances, []);
      const [unavailable] = yield* registry.listUnavailable;
      assert.include(unavailable?.unavailableReason ?? "", "invalid V1 instance");
      assert.deepStrictEqual(unavailable?.adapterPackage, ECHO_PACKAGE_REFERENCE);
    }).pipe(Effect.provide(testLayer), Effect.scoped),
  );

  it.effect("turns an unexpected session process exit into a canonical runtime error", () =>
    Effect.gen(function* () {
      const registryPath = yield* writeRegistry([registration]);
      const validPackage = makeEchoPackage();
      const threadId = ThreadId.make("crashing-harness");
      const crashingPackage: ExternalProviderAdapterPackageV1<Record<string, never>> = {
        ...validPackage,
        create: (input, host) =>
          validPackage.create(input, host).pipe(
            Effect.tap(() =>
              host.processes
                .spawn({
                  command: process.execPath,
                  purpose: { kind: "session", threadId },
                  args: ["-e", "process.exit(23)"],
                })
                .pipe(Effect.orDie),
            ),
          ),
      };
      const loaded = yield* loadTrustedProviderAdapterPackages({
        registryPath,
        reservedDriverKinds: new Set(),
        loadModule: async () => ({ default: crashingPackage }),
      });
      const { registry } = yield* makeProviderInstanceRegistry({
        drivers: loaded.drivers,
        configMap: registryConfig,
      });
      const instance = yield* registry.getInstance(ECHO_INSTANCE);
      assert.isDefined(instance);
      const event = yield* instance.adapter.streamEvents.pipe(Stream.runHead);

      assert.equal(event._tag, "Some");
      if (event._tag === "Some") {
        assert.equal(event.value.type, "runtime.error");
        assert.equal(event.value.threadId, threadId);
        assert.equal(event.value.providerInstanceId, ECHO_INSTANCE);
        if (event.value.type === "runtime.error") {
          assert.equal(event.value.payload.class, "transport_error");
          assert.include(event.value.payload.message, "code 23");
        }
      }
    }).pipe(Effect.provide(testLayer), Effect.scoped),
  );

  it.effect("rejects malformed canonical output from package methods", () =>
    Effect.gen(function* () {
      const registryPath = yield* writeRegistry([registration]);
      const validPackage = makeEchoPackage();
      const malformedPackage = {
        ...validPackage,
        create: (...args: Parameters<typeof validPackage.create>) =>
          validPackage.create(...args).pipe(
            Effect.map((instance) => ({
              ...instance,
              adapter: {
                ...instance.adapter,
                startSession: () => Effect.succeed({ status: "ready" }),
              },
            })),
          ),
      } as unknown as ExternalProviderAdapterPackageV1<Record<string, never>>;
      const loaded = yield* loadTrustedProviderAdapterPackages({
        registryPath,
        reservedDriverKinds: new Set(),
        loadModule: async () => ({ default: malformedPackage }),
      });
      const { registry } = yield* makeProviderInstanceRegistry({
        drivers: loaded.drivers,
        configMap: registryConfig,
      });
      const instance = yield* registry.getInstance(ECHO_INSTANCE);
      assert.isDefined(instance);
      const result = yield* instance.adapter
        .startSession({
          threadId: ThreadId.make("malformed-output"),
          providerInstanceId: ECHO_INSTANCE,
          runtimeMode: "full-access",
        })
        .pipe(Effect.result);

      assert.equal(result._tag, "Failure");
      if (result._tag === "Failure" && result.failure._tag === "ProviderAdapterRequestError") {
        assert.equal(result.failure.method, "startSession.decode");
      }
    }).pipe(Effect.provide(testLayer), Effect.scoped),
  );

  it.effect("fails an incompatible package closed and keeps its instance diagnosable", () =>
    Effect.gen(function* () {
      const registryPath = yield* writeRegistry([registration]);
      const loaded = yield* loadTrustedProviderAdapterPackages({
        registryPath,
        reservedDriverKinds: new Set(),
        loadModule: async () => ({
          default: makeEchoPackage(["turn.interrupt", "stream.reasoning"], {
            hostProtocol: { minimum: 3, maximum: 3 },
          }),
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
      const loaded = yield* loadTrustedProviderAdapterPackages({
        registryPath,
        reservedDriverKinds: new Set(),
        loadModule: async () => ({
          default: makeEchoPackage(["turn.interrupt", "stream.reasoning"], {
            capabilities: ["turn.interrupt"],
          }),
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
      const loaded = yield* loadTrustedProviderAdapterPackages({
        registryPath,
        reservedDriverKinds: new Set(),
      });

      assert.lengthOf(loaded.drivers, 1);
      assert.deepStrictEqual(loaded.diagnostics, []);
    }).pipe(Effect.provide(testLayer), Effect.scoped),
  );

  it.effect("keeps instances of disabled packages visible with a specific reason", () =>
    Effect.gen(function* () {
      const registryPath = yield* writeRegistry([{ ...registration, enabled: false }]);
      const loaded = yield* loadTrustedProviderAdapterPackages({
        registryPath,
        reservedDriverKinds: new Set(),
      });
      const { registry } = yield* makeProviderInstanceRegistry({
        drivers: loaded.drivers,
        unavailableDriverReasons: loaded.unavailableDriverReasons,
        configMap: registryConfig,
      });

      const [unavailable] = yield* registry.listUnavailable;
      assert.include(unavailable?.unavailableReason ?? "", "is disabled");
      assert.deepStrictEqual(unavailable?.adapterPackage, ECHO_PACKAGE_REFERENCE);
    }).pipe(Effect.provide(testLayer), Effect.scoped),
  );

  it.effect("rejects an external package that reuses a compiled package identity", () =>
    Effect.gen(function* () {
      const compiledPackageReference = {
        id: ProviderAdapterPackageId.make("prime-rpc"),
        version: ProviderAdapterPackageVersion.make("1.0.0"),
        protocolVersion: 1,
      } as const;
      const collidingRegistration = {
        ...registration,
        id: compiledPackageReference.id,
        version: compiledPackageReference.version,
      };
      const registryPath = yield* writeRegistry([collidingRegistration]);
      const loaded = yield* loadTrustedProviderAdapterPackages({
        registryPath,
        reservedDriverKinds: new Set(),
        reservedPackageReferences: [compiledPackageReference],
        loadModule: async () => ({
          default: makeEchoPackage(undefined, {
            id: compiledPackageReference.id,
            version: compiledPackageReference.version,
          }),
        }),
      });

      assert.deepStrictEqual(loaded.drivers, []);
      assert.deepStrictEqual(loaded.manifests, []);
      assert.equal(loaded.diagnostics[0]?.code, "package-conflict");
      assert.include(loaded.diagnostics[0]?.detail ?? "", "prime-rpc@1.0.0 (protocol 1)");
      assert.include(
        loaded.diagnostics[0]?.detail ?? "",
        "Choose a different package id or version",
      );
      assert.include(
        loaded.unavailableDriverReasons.get(ECHO_DRIVER) ?? "",
        "compiled package identity",
      );
    }).pipe(Effect.provide(testLayer), Effect.scoped),
  );

  it.effect("does not allow an external package to replace a built-in driver", () =>
    Effect.gen(function* () {
      const registryPath = yield* writeRegistry([registration]);
      const loaded = yield* loadTrustedProviderAdapterPackages({
        registryPath,
        reservedDriverKinds: new Set([ECHO_DRIVER]),
        loadModule: async () => ({
          default: makeEchoPackage(),
        }),
      });

      assert.deepStrictEqual(loaded.drivers, []);
      assert.equal(loaded.diagnostics[0]?.code, "driver-conflict");
    }).pipe(Effect.provide(testLayer), Effect.scoped),
  );
});
