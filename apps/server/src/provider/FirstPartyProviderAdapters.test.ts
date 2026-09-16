import { describe, expect, it } from "@effect/vitest";
import {
  ProviderDriverKind,
  ProviderInstanceId,
  ThreadId,
  type ProviderAdapterProtocolCapabilitiesV1,
  type ProviderSession,
  type ServerProvider,
} from "@t3tools/contracts";
import * as Effect from "effect/Effect";
import * as Schema from "effect/Schema";
import * as Stream from "effect/Stream";

import type { ProviderDriver, ProviderInstance } from "./ProviderDriver.ts";
import {
  makeFirstPartyProviderAdapterDriver,
  PRIME_PROVIDER_ADAPTER_MANIFEST,
  PRIME_PROVIDER_ADAPTER_PACKAGE_REFERENCE,
} from "./FirstPartyProviderAdapters.ts";
import { makeManualOnlyProviderMaintenanceCapabilities } from "./providerMaintenance.ts";

const DRIVER_KIND = ProviderDriverKind.make("primeAgent");
const INSTANCE_ID = ProviderInstanceId.make("prime_test");
const THREAD_ID = ThreadId.make("00000000-0000-4000-8000-000000000001");
const NOW = "2026-09-16T00:00:00.000Z";

const makeCompiledDriver = (
  capabilities: ProviderAdapterProtocolCapabilitiesV1,
): ProviderDriver<Record<string, never>> => ({
  driverKind: DRIVER_KIND,
  metadata: { displayName: "Prime Agent", supportsMultipleInstances: true },
  configSchema: Schema.Struct({}),
  defaultConfig: () => ({}),
  create: ({ instanceId, displayName, accentColor, enabled }) => {
    const session: ProviderSession = {
      provider: DRIVER_KIND,
      status: "ready",
      runtimeMode: "full-access",
      threadId: THREAD_ID,
      createdAt: NOW,
      updatedAt: NOW,
    };
    const baseSnapshot: ServerProvider = {
      instanceId,
      driver: DRIVER_KIND,
      displayName,
      accentColor,
      enabled,
      installed: true,
      version: "1.0.0",
      status: "ready",
      auth: { status: "authenticated" },
      checkedAt: NOW,
      models: [],
      slashCommands: [],
      skills: [],
    };
    const adapter = {
      provider: DRIVER_KIND,
      capabilities: { sessionModelSwitch: "in-session", protocol: capabilities },
      startSession: () => Effect.succeed(session),
      listSessions: () => Effect.succeed([session]),
      streamEvents: Stream.empty,
    } as unknown as ProviderInstance["adapter"];

    return Effect.succeed({
      instanceId,
      driverKind: DRIVER_KIND,
      continuationIdentity: {
        driverKind: DRIVER_KIND,
        continuationKey: `${DRIVER_KIND}:instance:${instanceId}`,
      },
      displayName,
      accentColor,
      enabled,
      adapter,
      snapshot: {
        maintenanceCapabilities: makeManualOnlyProviderMaintenanceCapabilities({
          provider: DRIVER_KIND,
          packageName: null,
        }),
        getSnapshot: Effect.succeed(baseSnapshot),
        refresh: Effect.succeed(baseSnapshot),
        streamChanges: Stream.make(baseSnapshot),
      },
      textGeneration: {} as ProviderInstance["textGeneration"],
    } satisfies ProviderInstance);
  },
});

const createInput = {
  instanceId: INSTANCE_ID,
  displayName: "Prime Test",
  accentColor: "#123456",
  environment: [],
  enabled: true,
  config: {},
} as const;

describe("first-party provider adapter bridge", () => {
  it("declares the exact negotiated Prime capability ceiling", () => {
    expect(PRIME_PROVIDER_ADAPTER_MANIFEST.capabilities).toEqual([
      "session.resume",
      "turn.steer",
      "turn.interrupt",
      "input.attachments",
      "request.approval",
      "request.structured-input",
      "model.discovery",
      "model.switch",
      "reasoning.selection",
      "stream.reasoning",
      "stream.tool-lifecycle",
      "stream.usage",
      "stream.subagents",
    ]);
  });

  it.effect("stamps the Prime package identity, config schema, and negotiated capabilities", () =>
    Effect.gen(function* () {
      const driver = makeFirstPartyProviderAdapterDriver({
        manifest: PRIME_PROVIDER_ADAPTER_MANIFEST,
        driver: makeCompiledDriver({
          protocolVersion: 1,
          features: ["session.resume", "turn.interrupt"],
        }),
      });
      const instance = yield* driver.create(createInput);

      expect(driver.adapterPackage).toEqual(PRIME_PROVIDER_ADAPTER_PACKAGE_REFERENCE);
      expect(instance.adapterPackage).toEqual(PRIME_PROVIDER_ADAPTER_PACKAGE_REFERENCE);
      expect(instance.adapter.adapterPackage).toEqual(PRIME_PROVIDER_ADAPTER_PACKAGE_REFERENCE);

      const snapshot = yield* instance.snapshot.getSnapshot;
      expect(snapshot.adapterPackage).toEqual(PRIME_PROVIDER_ADAPTER_PACKAGE_REFERENCE);
      expect(snapshot.adapterConfigSchema).toEqual(PRIME_PROVIDER_ADAPTER_MANIFEST.configSchema);
      expect(snapshot.adapterCapabilities).toEqual({
        protocolVersion: 1,
        features: ["session.resume", "turn.interrupt"],
      });

      const started = yield* instance.adapter.startSession({
        threadId: THREAD_ID,
        providerInstanceId: INSTANCE_ID,
        runtimeMode: "full-access",
      });
      expect(started.providerInstanceId).toBe(INSTANCE_ID);
      expect(started.adapterPackage).toEqual(PRIME_PROVIDER_ADAPTER_PACKAGE_REFERENCE);

      const listed = yield* instance.adapter.listSessions();
      expect(listed[0]?.adapterPackage).toEqual(PRIME_PROVIDER_ADAPTER_PACKAGE_REFERENCE);
    }).pipe(Effect.scoped),
  );

  it.effect("rejects negotiated capabilities absent from the first-party manifest", () => {
    const driver = makeFirstPartyProviderAdapterDriver({
      manifest: PRIME_PROVIDER_ADAPTER_MANIFEST,
      driver: makeCompiledDriver({ protocolVersion: 1, features: ["feedback.upload"] }),
    });

    return driver.create(createInput).pipe(
      Effect.flip,
      Effect.tap((error) =>
        Effect.sync(() => {
          expect(error.detail).toContain("undeclared capabilities");
          expect(error.detail).toContain("feedback.upload");
        }),
      ),
      Effect.scoped,
    );
  });
});
