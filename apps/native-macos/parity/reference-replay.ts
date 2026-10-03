// @effect-diagnostics nodeBuiltinImport:off - The differential adapter reads stdin synchronously before running the pure production reducer.
import * as NodeFS from "node:fs";
import * as Schema from "effect/Schema";
import { OrchestrationEvent, OrchestrationThread } from "@t3tools/contracts";
import { applyThreadDetailEvent } from "../../../packages/client-runtime/src/state/threadReducer.ts";

// Exercise the current client's production reducer; do not reproduce its logic here.
const input = Schema.decodeUnknownSync(
  Schema.Struct({
    thread: OrchestrationThread,
    events: Schema.Array(OrchestrationEvent),
  }),
)(JSON.parse(NodeFS.readFileSync(0, "utf8")));
let thread: OrchestrationThread | null = input.thread;
const frames = [];
for (const event of input.events) {
  if (thread === null) throw new Error("Event after thread deletion");
  const result = applyThreadDetailEvent(thread, event);
  if (result.kind === "deleted") thread = null;
  else if (result.kind === "updated") thread = result.thread;
  frames.push(thread);
}
process.stdout.write(JSON.stringify({ normalized_input: input, frames }) + "\n");
