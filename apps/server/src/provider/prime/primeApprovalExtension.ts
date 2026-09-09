import * as Effect from "effect/Effect";
import * as FileSystem from "effect/FileSystem";
import * as Path from "effect/Path";
import * as Schema from "effect/Schema";

export const PRIME_APPROVAL_EXTENSION_PROTOCOL_VERSION = 1 as const;
export const PRIME_APPROVAL_EXTENSION_COMMAND_NAME = "t3-approval-v1" as const;
export const PRIME_APPROVAL_EXTENSION_COMMAND_DESCRIPTION =
  "T3 approval bridge protocol 1" as const;
export const PRIME_APPROVAL_EXTENSION_MODE_FLAG = "t3-approval-mode" as const;

const PRIME_APPROVAL_EXTENSION_FILE_NAME = "t3-approval-v1.ts";

const PrimeApprovalExtensionHandshake = Schema.Struct({
  commands: Schema.Array(
    Schema.Struct({
      name: Schema.String,
      description: Schema.optional(Schema.String),
      source: Schema.String,
      sourceInfo: Schema.Struct({
        path: Schema.String,
        source: Schema.String,
        scope: Schema.String,
        origin: Schema.String,
      }),
    }),
  ),
});

const isPrimeApprovalExtensionHandshakeData = Schema.is(PrimeApprovalExtensionHandshake);

export function isPrimeApprovalExtensionHandshake(data: unknown, extensionPath: string): boolean {
  if (!isPrimeApprovalExtensionHandshakeData(data)) {
    return false;
  }
  const matching = data.commands.filter(
    (command) => command.name === PRIME_APPROVAL_EXTENSION_COMMAND_NAME,
  );
  const command = matching[0];
  return (
    matching.length === 1 &&
    command !== undefined &&
    command.description === PRIME_APPROVAL_EXTENSION_COMMAND_DESCRIPTION &&
    command.source === "extension" &&
    command.sourceInfo.path === extensionPath &&
    command.sourceInfo.source === "cli" &&
    command.sourceInfo.scope === "temporary" &&
    command.sourceInfo.origin === "top-level"
  );
}

const PRIME_APPROVAL_EXTENSION_SOURCE = `import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export default function t3ApprovalExtension(pi: ExtensionAPI) {
  pi.registerFlag("t3-approval-mode", {
    description: "T3-owned approval bridge mode",
    type: "string",
    default: "deny",
  });
  pi.registerCommand("t3-approval-v1", {
    description: "T3 approval bridge protocol 1",
    handler: () => {},
  });

  pi.on("tool_call", async (event, ctx) => {
    const mode = pi.getFlag("t3-approval-mode");
    if (mode === "full-access") {
      return undefined;
    }
    if (mode !== "approval-required") {
      return { block: true, reason: "T3 approval bridge mode is missing or invalid" };
    }
    if (!ctx.hasUI) {
      return { block: true, reason: "T3 approval UI is unavailable" };
    }

    let input = "[unserializable input]";
    try {
      input = JSON.stringify(event.input, null, 2) ?? input;
    } catch {}
    const allowed = await ctx.ui.confirm(
      "Allow Prime Agent tool?",
      event.toolName + "\\n\\n" + input,
      { signal: ctx.signal },
    );
    if (!allowed) {
      return { block: true, reason: "Rejected by user" };
    }
    return undefined;
  });
}
`;

export const preparePrimeApprovalExtension = Effect.fn("preparePrimeApprovalExtension")(function* (
  baseDir: string,
) {
  const fileSystem = yield* FileSystem.FileSystem;
  const path = yield* Path.Path;
  const extensionDirectory = path.resolve(baseDir, "prime-agent", "extensions");
  const extensionPath = path.join(extensionDirectory, PRIME_APPROVAL_EXTENSION_FILE_NAME);

  yield* fileSystem.makeDirectory(extensionDirectory, { recursive: true });
  yield* fileSystem.writeFileString(extensionPath, PRIME_APPROVAL_EXTENSION_SOURCE);

  return extensionPath;
});
