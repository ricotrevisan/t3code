// @effect-diagnostics nodeBuiltinImport:off
import { expect, it } from "@effect/vitest";
import * as NodeFS from "node:fs";
import * as NodePath from "node:path";
import * as NodeServices from "@effect/platform-node/NodeServices";
import {
  ProviderAdapterPackageId,
  ProviderAdapterPackageVersion,
  ProviderDriverKind,
  ProviderInstanceId,
} from "@t3tools/contracts";
import { defineProviderAdapterV1, type ProviderAdapterPackageV1 } from "@t3tools/provider-adapter";
import { symlinksSupported } from "@t3tools/shared/testing/symlinks";
import * as Effect from "effect/Effect";
import * as FileSystem from "effect/FileSystem";
import * as Layer from "effect/Layer";
import * as Schema from "effect/Schema";
import * as Stream from "effect/Stream";
import { HttpClient, HttpClientResponse } from "effect/unstable/http";

import { ServerConfig } from "../config.ts";
import { ServerSettingsService } from "../serverSettings.ts";
import { makeExternalProviderDriver } from "./ExternalProviderDriver.ts";

const DRIVER = ProviderDriverKind.make("maintainedHarness");
const INSTANCE = ProviderInstanceId.make("maintained_harness");
const NOW = "2026-09-19T00:00:00.000Z";

interface MaintainedHarnessConfig {
  readonly binaryPath: string;
}

const makePackage = (): ProviderAdapterPackageV1<MaintainedHarnessConfig> =>
  defineProviderAdapterV1({
    manifest: {
      protocolVersion: 1,
      id: ProviderAdapterPackageId.make("maintained-harness"),
      version: ProviderAdapterPackageVersion.make("1.0.0"),
      driver: DRIVER,
      displayName: "Maintained Harness",
      hostProtocol: { minimum: 1, maximum: 1 },
      transport: {
        kind: "supervised-stdio",
        protocol: "jsonl-rpc",
        sessionConcurrency: "one-per-process",
      },
      capabilities: [],
      maintenance: {
        npmPackage: "@example/maintained-harness",
        binaryConfigKey: "binaryPath",
      },
      configSchema: {
        type: "object",
        additionalProperties: false,
        required: ["binaryPath"],
        properties: { binaryPath: { type: "string" } },
      },
    },
    configSchema: Schema.Struct({ binaryPath: Schema.String }),
    defaultConfig: () => ({ binaryPath: "maintained-harness" }),
    create: ({ instanceId, displayName, enabled }) => {
      const snapshot = {
        instanceId,
        driver: DRIVER,
        displayName,
        enabled,
        installed: true,
        // External packages may report their adapter version. The server must
        // replace it with the owning installer's harness version.
        version: "1.0.0",
        status: "ready" as const,
        auth: { status: "unknown" as const },
        checkedAt: NOW,
        availability: "available" as const,
        models: [],
        slashCommands: [],
        skills: [],
      };
      const unsupported = () => Effect.die("not exercised by maintenance tests");
      return Effect.succeed({
        snapshot: {
          getSnapshot: Effect.succeed(snapshot),
          refresh: Effect.succeed(snapshot),
          streamChanges: Stream.empty,
        },
        adapter: {
          capabilities: { protocolVersion: 1 as const, features: [] },
          startSession: unsupported,
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

const TestHttpClientLive = Layer.succeed(
  HttpClient.HttpClient,
  HttpClient.make((request) =>
    Effect.succeed(HttpClientResponse.fromWeb(request, Response.json({ version: "2.0.0" }))),
  ),
);

const testLayer = ServerConfig.layerTest(process.cwd(), {
  prefix: "external-provider-maintenance-test-",
}).pipe(
  Layer.provideMerge(NodeServices.layer),
  Layer.provideMerge(ServerSettingsService.layerTest()),
  Layer.provideMerge(TestHttpClientLive),
);

it.layer(testLayer)("external provider maintenance", (it) => {
  it.effect.skipIf(!symlinksSupported)(
    "derives the harness version, advisory, and update command from its owning npm prefix",
    () =>
      Effect.gen(function* () {
        const fileSystem = yield* FileSystem.FileSystem;
        const tempDir = yield* fileSystem.makeTempDirectoryScoped({
          prefix: "external-provider-npm-",
        });
        const packageDir = NodePath.join(
          tempDir,
          "lib",
          "node_modules",
          "@example",
          "maintained-harness",
        );
        const entryPoint = NodePath.join(packageDir, "bin", "maintained-harness.js");
        NodeFS.mkdirSync(NodePath.dirname(entryPoint), { recursive: true });
        NodeFS.writeFileSync(entryPoint, "#!/bin/sh\n");
        NodeFS.chmodSync(entryPoint, 0o755);
        NodeFS.writeFileSync(NodePath.join(packageDir, "package.json"), '{"version":"1.2.3"}');
        const binaryPath = NodePath.join(tempDir, "bin", "maintained-harness");
        NodeFS.mkdirSync(NodePath.dirname(binaryPath), { recursive: true });
        NodeFS.symlinkSync(entryPoint, binaryPath);
        const realTempDir = NodeFS.realpathSync(tempDir);

        const adapterPackage = makePackage();
        const driver = makeExternalProviderDriver(adapterPackage, adapterPackage.defaultConfig());
        const instance = yield* driver.create({
          instanceId: INSTANCE,
          displayName: "Maintained Harness",
          environment: [{ name: "PATH", value: NodePath.dirname(binaryPath), sensitive: false }],
          enabled: true,
          config: { binaryPath: "maintained-harness" },
        });

        const maintenance = yield* instance.snapshot.resolveMaintenance();
        expect(maintenance).toMatchObject({
          packageName: "@example/maintained-harness",
          installedVersion: "1.2.3",
          update: {
            executable: "npm",
            args: expect.arrayContaining([
              "--prefix",
              realTempDir,
              "@example/maintained-harness@latest",
            ]),
          },
        });

        const snapshot = yield* instance.snapshot.getSnapshot;
        expect(snapshot.version).toBe("1.2.3");
        expect(snapshot.versionAdvisory).toMatchObject({
          status: "behind_latest",
          currentVersion: "1.2.3",
          latestVersion: "2.0.0",
          canUpdate: true,
        });
      }),
  );

  it.effect(
    "does not publish the adapter package version for an unrecognized harness install",
    () =>
      Effect.gen(function* () {
        const fileSystem = yield* FileSystem.FileSystem;
        const tempDir = yield* fileSystem.makeTempDirectoryScoped({
          prefix: "external-provider-custom-",
        });
        const binaryPath = NodePath.join(tempDir, "maintained-harness");
        NodeFS.writeFileSync(binaryPath, "#!/bin/sh\n");
        NodeFS.chmodSync(binaryPath, 0o755);

        const adapterPackage = makePackage();
        const driver = makeExternalProviderDriver(adapterPackage, adapterPackage.defaultConfig());
        const instance = yield* driver.create({
          instanceId: INSTANCE,
          displayName: "Maintained Harness",
          environment: [],
          enabled: true,
          config: { binaryPath },
        });

        const maintenance = yield* instance.snapshot.resolveMaintenance();
        expect(maintenance.update).toBeNull();

        const snapshot = yield* instance.snapshot.getSnapshot;
        expect(snapshot.version).toBeNull();
        expect(snapshot.versionAdvisory).toMatchObject({
          status: "unknown",
          currentVersion: null,
          latestVersion: null,
          canUpdate: false,
        });
      }),
  );
});
