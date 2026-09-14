import * as Effect from "effect/Effect";
import * as Schema from "effect/Schema";

export default {
  manifest: {
    protocolVersion: 1,
    id: "echo-adapter",
    version: "1.0.0",
    driver: "echoHarness",
    displayName: "Echo Adapter",
    hostProtocol: { minimum: 1, maximum: 1 },
    transport: {
      kind: "supervised-stdio",
      protocol: "jsonl-rpc",
      sessionConcurrency: "one-per-process",
    },
    capabilities: ["turn.interrupt", "stream.reasoning"],
    configSchema: { type: "object", additionalProperties: false },
  },
  driver: {
    driverKind: "echoHarness",
    metadata: { displayName: "Echo Adapter" },
    configSchema: Schema.Struct({}),
    defaultConfig: () => ({}),
    create: () => Effect.die("not exercised by dynamic import test"),
  },
};
