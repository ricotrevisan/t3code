import type * as Effect from "effect/Effect";

import type { ExternalProviderDriverEnv } from "../ExternalProviderDriver.ts";

export interface AcpConformanceCheck {
  readonly name: string;
  readonly ok: boolean;
  readonly detail?: string | undefined;
}

export interface AcpConformanceResult {
  readonly checks: ReadonlyArray<AcpConformanceCheck>;
  readonly eventCount: number;
}

export const runAcpConformance: Effect.Effect<
  AcpConformanceResult,
  Error,
  ExternalProviderDriverEnv
>;
