import * as Effect from "effect/Effect";
import * as FileSystem from "effect/FileSystem";
import * as Path from "effect/Path";

// Keep this standalone: Prime loads it from T3 home, outside the server package.
const PRIME_USER_INPUT_EXTENSION_SOURCE = `import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import type { TUnsafe } from "typebox";

type Input = { question: string; options?: string[] };

export default function t3UserInputExtension(pi: ExtensionAPI) {
  pi.registerTool({
    name: "request_user_input",
    label: "Ask user",
    description: "Ask the user a question and wait for their answer. Supply options for a single-choice question, or omit options for free text. Ask one question per call. Cancellation is not an answer.",
    parameters: {
      type: "object",
      properties: {
        question: { type: "string", minLength: 1, description: "The question to show the user." },
        options: { type: "array", minItems: 1, uniqueItems: true, items: { type: "string", minLength: 1 }, description: "Optional choices for a single-choice question." },
      },
      required: ["question"],
      additionalProperties: false,
    } as TUnsafe<Input>,
    async execute(_toolCallId: string, params: Input, signal, _onUpdate, ctx) {
      if (!ctx.hasUI) {
        throw new Error("T3 user input UI is unavailable");
      }
      const question = params.question.trim();
      const options = params.options?.map((option) => option.trim());
      if (!question || (options && (!options.length || options.some((option) => !option) || new Set(options).size !== options.length))) {
        throw new Error("Provide a nonempty question and distinct, nonempty options");
      }
      const answer = signal?.aborted ? undefined : options
        ? await ctx.ui.select(question, options, { signal })
        : await ctx.ui.input(question, undefined, { signal });
      const details = answer === undefined || signal?.aborted
        ? { cancelled: true }
        : { cancelled: false, answer };
      return { content: [{ type: "text", text: JSON.stringify(details) }], details };
    },
  });
}
`;

export const preparePrimeUserInputExtension = Effect.fn("preparePrimeUserInputExtension")(
  function* (baseDir: string) {
    const fileSystem = yield* FileSystem.FileSystem;
    const path = yield* Path.Path;
    const directory = path.resolve(baseDir, "prime-agent", "extensions");
    const extensionPath = path.join(directory, "t3-user-input.ts");
    yield* fileSystem.makeDirectory(directory, { recursive: true });
    yield* fileSystem.writeFileString(extensionPath, PRIME_USER_INPUT_EXTENSION_SOURCE);
    return extensionPath;
  },
);
