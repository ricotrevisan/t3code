import type * as Effect from "effect/Effect";

import type { ExternalProviderDriverEnv } from "../ExternalProviderDriver.ts";

export interface PiConformanceCheck {
  readonly name: string;
  readonly ok: boolean;
  readonly detail?: string | undefined;
}

export interface PiConformanceResult {
  readonly checks: ReadonlyArray<PiConformanceCheck>;
  readonly eventCount: number;
}

export const runPiConformance: Effect.Effect<PiConformanceResult, Error, ExternalProviderDriverEnv>;
