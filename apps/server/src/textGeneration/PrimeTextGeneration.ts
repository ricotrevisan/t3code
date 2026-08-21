import { TextGenerationError } from "@t3tools/contracts";
import * as Effect from "effect/Effect";

import * as TextGeneration from "./TextGeneration.ts";

const failClosed = (operation: string) =>
  Effect.fail(
    new TextGenerationError({
      operation,
      detail: "Prime Agent cannot generate text yet.",
    }),
  );

export const makePrimeTextGeneration = (): Effect.Effect<
  TextGeneration.TextGeneration["Service"]
> =>
  Effect.succeed({
    generateCommitMessage: (_input) => failClosed("generateCommitMessage"),
    generatePrContent: (_input) => failClosed("generatePrContent"),
    generateBranchName: (_input) => failClosed("generateBranchName"),
    generateThreadTitle: (_input) => failClosed("generateThreadTitle"),
  });
