import { describe, expect, it } from "@effect/vitest";
import { ProviderInstanceId } from "@t3tools/contracts";
import * as Effect from "effect/Effect";
import * as Result from "effect/Result";

import { makePrimeTextGeneration } from "./PrimeTextGeneration.ts";

const modelSelection = {
  instanceId: ProviderInstanceId.make("primeAgent"),
  model: "unused",
};

describe("PrimeTextGeneration", () => {
  it.effect("fails closed for thread title generation", () =>
    Effect.gen(function* () {
      const textGeneration = yield* makePrimeTextGeneration();
      const result = yield* textGeneration
        .generateThreadTitle({
          cwd: process.cwd(),
          message: "hello",
          modelSelection,
        })
        .pipe(Effect.result);

      expect(Result.isFailure(result)).toBe(true);
      if (Result.isFailure(result)) {
        expect(result.failure.message).toContain("cannot generate text yet");
      }
    }),
  );
});
