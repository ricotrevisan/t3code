/**
 * Compiled provider adapter package registrations shipped by this server.
 *
 * These packages use the public adapter host, just like trusted-local
 * packages, but are linked into the server build and reserve their package
 * and driver identities.
 *
 * @module provider/FirstPartyProviderAdapters
 */
import { DEFAULT_PROVIDER_HEALTH_REFRESH_INTERVAL } from "@t3tools/contracts";
import {
  PI_PROVIDER_ADAPTER_DEFAULT_CONFIG,
  PI_PROVIDER_ADAPTER_MANIFEST,
  PI_PROVIDER_ADAPTER_PACKAGE,
  PI_PROVIDER_ADAPTER_PACKAGE_REFERENCE,
} from "@t3tools/provider-adapter-pi";
import {
  PRIME_PROVIDER_ADAPTER_DEFAULT_CONFIG,
  PRIME_PROVIDER_ADAPTER_MANIFEST,
  PRIME_PROVIDER_ADAPTER_PACKAGE,
  PRIME_PROVIDER_ADAPTER_PACKAGE_REFERENCE,
} from "@t3tools/provider-adapter-prime";
import { resolveServerBackgroundActivitySettings } from "@t3tools/shared/backgroundActivitySettings";
import * as Duration from "effect/Duration";
import * as Effect from "effect/Effect";
import * as Queue from "effect/Queue";
import * as Stream from "effect/Stream";

import * as BackgroundPolicy from "../background/BackgroundPolicy.ts";
import { ServerSettingsService } from "../serverSettings.ts";
import {
  DEEPSEEK_HARNESS_ADAPTER_DEFAULT_CONFIG,
  DEEPSEEK_HARNESS_ADAPTER_PACKAGE,
} from "./DeepSeekHarnessAdapter.ts";
import {
  makeExternalProviderDriver,
  type ExternalProviderDriverEnv,
} from "./ExternalProviderDriver.ts";
import type { AnyProviderDriver, ProviderInstance } from "./ProviderDriver.ts";

export {
  PI_PROVIDER_ADAPTER_MANIFEST,
  PI_PROVIDER_ADAPTER_PACKAGE_REFERENCE,
  PRIME_PROVIDER_ADAPTER_MANIFEST,
  PRIME_PROVIDER_ADAPTER_PACKAGE_REFERENCE,
};

export type FirstPartyProviderAdaptersEnv =
  | ExternalProviderDriverEnv
  | BackgroundPolicy.BackgroundPolicy
  | ServerSettingsService;

const startDemandAwareSnapshotRefresh = Effect.fn("startDemandAwareSnapshotRefresh")(function* (
  instance: ProviderInstance,
) {
  const backgroundPolicy = yield* BackgroundPolicy.BackgroundPolicy;
  const serverSettings = yield* ServerSettingsService;
  const intervalChanges = yield* Queue.sliding<void>(1);
  const settingsChanges = yield* serverSettings.subscribeChanges;

  yield* settingsChanges.pipe(
    Stream.map((settings) =>
      Duration.toMillis(
        resolveServerBackgroundActivitySettings(settings).providerHealthRefreshInterval,
      ),
    ),
    Stream.changes,
    Stream.runForEach(() => Queue.offer(intervalChanges, undefined).pipe(Effect.asVoid)),
    Effect.forkScoped,
  );

  const getRefreshInterval = serverSettings.getSettings.pipe(
    Effect.map(
      (settings) => resolveServerBackgroundActivitySettings(settings).providerHealthRefreshInterval,
    ),
    Effect.orElseSucceed(() => DEFAULT_PROVIDER_HEALTH_REFRESH_INTERVAL),
  );
  const hasDemand = Effect.all([
    backgroundPolicy.shouldRunScopeWork({ type: "provider-status" }),
    backgroundPolicy.shouldRunScopeWork({
      type: "provider-status",
      instanceId: instance.instanceId,
    }),
  ]).pipe(Effect.map(([genericDemand, instanceDemand]) => genericDemand || instanceDemand));

  yield* Effect.forever(
    getRefreshInterval.pipe(
      Effect.flatMap((refreshInterval) =>
        Effect.raceFirst(
          Effect.sleep(
            Duration.toMillis(Duration.fromInputUnsafe(refreshInterval)) <= 0
              ? "60 seconds"
              : refreshInterval,
          ).pipe(Effect.as(true)),
          Queue.take(intervalChanges).pipe(Effect.as(false)),
        ).pipe(
          Effect.flatMap((elapsed) =>
            elapsed && Duration.toMillis(Duration.fromInputUnsafe(refreshInterval)) > 0
              ? hasDemand.pipe(
                  Effect.flatMap((shouldRefresh) =>
                    shouldRefresh ? instance.snapshot.refresh.pipe(Effect.asVoid) : Effect.void,
                  ),
                )
              : Effect.void,
          ),
        ),
      ),
      Effect.ignoreCause({ log: true }),
    ),
  ).pipe(Effect.forkScoped);
});

const withDemandAwareSnapshotRefresh = (
  driver: AnyProviderDriver<ExternalProviderDriverEnv>,
): AnyProviderDriver<FirstPartyProviderAdaptersEnv> => ({
  ...driver,
  create: (input) =>
    driver.create(input).pipe(Effect.tap((instance) => startDemandAwareSnapshotRefresh(instance))),
});

/** Compiled package registrations shipped by this build. */
export const FIRST_PARTY_PROVIDER_ADAPTER_DRIVERS: ReadonlyArray<
  AnyProviderDriver<FirstPartyProviderAdaptersEnv>
> = [
  withDemandAwareSnapshotRefresh(
    makeExternalProviderDriver(
      PRIME_PROVIDER_ADAPTER_PACKAGE,
      PRIME_PROVIDER_ADAPTER_DEFAULT_CONFIG,
    ),
  ),
  makeExternalProviderDriver(
    DEEPSEEK_HARNESS_ADAPTER_PACKAGE,
    DEEPSEEK_HARNESS_ADAPTER_DEFAULT_CONFIG,
  ),
  makeExternalProviderDriver(PI_PROVIDER_ADAPTER_PACKAGE, PI_PROVIDER_ADAPTER_DEFAULT_CONFIG),
];
