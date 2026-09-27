import { NodeHttpServer } from "@effect/platform-node";
import * as NodeServices from "@effect/platform-node/NodeServices";
import { expect, it } from "@effect/vitest";
import {
  EnvironmentId,
  ProviderInstanceId,
  ProviderAdapterPackageId,
  ThreadId,
} from "@t3tools/contracts";
import * as Deferred from "effect/Deferred";
import * as Effect from "effect/Effect";
import * as FileSystem from "effect/FileSystem";
import * as Layer from "effect/Layer";
import * as Path from "effect/Path";
import * as Queue from "effect/Queue";
import * as Schema from "effect/Schema";
import * as Stream from "effect/Stream";
import { HttpRouter } from "effect/unstable/http";
import { PI_PREVIEW_EXTENSION_SOURCE } from "../../../../packages/provider-adapter-pi/src/piPreviewExtension.ts";
import { ServerConfig } from "../config.ts";
import { ServerEnvironment } from "../environment/ServerEnvironment.ts";
import { DeviceService } from "../device/DeviceService.ts";
import { OrchestrationEngineService } from "../orchestration/Services/OrchestrationEngine.ts";
import { ProjectionSnapshotQuery } from "../orchestration/Services/ProjectionSnapshotQuery.ts";
import * as McpHttpServer from "../mcp/McpHttpServer.ts";
import * as McpSessionRegistry from "../mcp/McpSessionRegistry.ts";
import * as PreviewAutomationBroker from "../mcp/PreviewAutomationBroker.ts";
import {
  makePiAdapter,
  piLaunchArgs,
} from "../../../../packages/provider-adapter-pi/src/PiAdapter.ts";
import { makeExternalProviderAdapterHostV2 } from "./ExternalProviderAdapterHost.ts";
import { makeExternalProviderProcessSupervisor } from "./ExternalProviderProcessSupervisor.ts";

const environmentId = EnvironmentId.make("pi-preview-smoke");
const threadId = ThreadId.make("pi-preview-smoke-thread");
const layer = Layer.mergeAll(McpSessionRegistry.layer, PreviewAutomationBroker.layer).pipe(
  Layer.provideMerge(
    Layer.succeed(ServerEnvironment, {
      getEnvironmentId: Effect.succeed(environmentId),
      getDescriptor: Effect.die("unused"),
    }),
  ),
  Layer.provideMerge(NodeHttpServer.layerTest),
  Layer.provideMerge(ServerConfig.layerTest(process.cwd(), { prefix: "pi-preview-smoke-" })),
  Layer.provideMerge(NodeServices.layer),
);
const serverLayer = McpHttpServer.layer.pipe(
  Layer.provide(
    Layer.mergeAll(
      Layer.mock(DeviceService)({}),
      Layer.mock(OrchestrationEngineService)({}),
      Layer.mock(ProjectionSnapshotQuery)({}),
    ),
  ),
);
const encodeJson = Schema.encodeSync(Schema.fromJsonString(Schema.Unknown));
const decodeResult = Schema.decodeUnknownSync(
  Schema.fromJsonString(
    Schema.Struct({
      names: Schema.Array(Schema.String),
      status: Schema.String,
      opened: Schema.String,
      image: Schema.Boolean,
    }),
  ),
);

// Opt-in: exercises an installed Pi, with its own config/cwd/session directories.
// The broker host is synthetic; this verifies RPC -> real MCP -> broker, not UI readiness.
it.live(
  "Pi RPC preserves preview tools and hides bootstrap env from configured/discovered factories after resume",
  () =>
    Effect.gen(function* () {
      yield* HttpRouter.serve(serverLayer, { disableListenLog: true, disableLogger: true }).pipe(
        Layer.build,
      );
      const registry = yield* McpSessionRegistry.McpSessionRegistry;
      const credential = yield* registry.issue({
        threadId,
        providerInstanceId: ProviderInstanceId.make("piRpc_smoke"),
        capabilities: new Set(["preview"]),
      });
      const broker = yield* PreviewAutomationBroker.PreviewAutomationBroker;
      const connected = yield* Deferred.make<void>();
      const events = yield* broker.connect({ clientId: "synthetic-pi-smoke-host", environmentId });
      const operations: string[] = [];
      yield* Stream.runForEach(events, (event) => {
        if (event.type === "connected") return Deferred.succeed(connected, undefined);
        operations.push(event.request.operation);
        const result =
          event.request.operation === "snapshot"
            ? {
                url: "https://example.test/",
                title: "Isolated fixture",
                loading: false,
                visibleText: "Fixture",
                interactiveElements: [],
                accessibilityTree: {},
                consoleEntries: [],
                networkEntries: [],
                actionTimeline: [],
                screenshot: { mimeType: "image/png", data: "cG5n", width: 1, height: 1 },
              }
            : {
                tabId: "smoke-tab",
                available: true,
                visible: event.request.operation === "open",
                loading: false,
                url: "https://example.test/",
                title: "Isolated fixture",
              };
        return broker.respond({
          clientId: "synthetic-pi-smoke-host",
          connectionId: event.connectionId,
          requestId: event.request.requestId,
          ok: true,
          result,
        });
      }).pipe(Effect.forkScoped);
      yield* Deferred.await(connected);

      const fs = yield* FileSystem.FileSystem;
      const path = yield* Path.Path;
      const directory = yield* fs.makeTempDirectoryScoped({ prefix: "t3-pi-rpc-smoke-" });
      const bridgePath = path.join(directory, "bridge.mjs");
      yield* fs.writeFileString(bridgePath, PI_PREVIEW_EXTENSION_SOURCE);
      // Capture registered definitions to invoke them through a real Pi RPC slash
      // command without needing model credentials or making a paid model request.
      const wrapper = `import { writeSync } from "node:fs";
import bridge from "./bridge.mjs";
export default async function(pi) {
  const tools = new Map();
  await bridge(new Proxy(pi, { get(target, key) { return key === "registerTool" ? tool => { tools.set(tool.name, tool); target.registerTool(tool); } : target[key]; } }));
  pi.registerCommand("t3-preview-smoke", { description: "Run isolated preview smoke", handler: async () => {
    try {
      const status = await tools.get("preview_status").execute("status", {});
      const opened = await tools.get("preview_open").execute("open", {});
      const snapshot = await tools.get("preview_snapshot").execute("snapshot", {});
      writeSync(1, "T3_SMOKE " + JSON.stringify({ names: [...tools.keys()], status: status.content[0].text, opened: opened.content[0].text, image: snapshot.content.some(b => b.type === "image") }) + "\\n");
    } catch (e) { writeSync(1, "T3_SMOKE_ERROR " + String(e) + "\\n"); }
  }});
}`;
      const observerPath = path.join(directory, "observer.mjs");
      const observationsPath = path.join(directory, "observations.jsonl");
      const observer = `import { appendFileSync } from "node:fs";
export default function() {
  appendFileSync(${encodeJson(observationsPath)}, JSON.stringify({ exposed: Boolean(process.env.T3_PI_MCP_AUTHORIZATION || process.env.T3_PI_MCP_ENDPOINT) }) + "\\n");
}`;
      yield* fs.writeFileString(observerPath, observer);
      const discoveredDirectory = path.join(directory, "agent", "extensions");
      yield* fs.makeDirectory(discoveredDirectory, { recursive: true });
      yield* fs.writeFileString(path.join(discoveredDirectory, "observer.ts"), observer);
      const wrapperPath = path.join(directory, "smoke.mjs");
      yield* fs.writeFileString(wrapperPath, wrapper);
      const supervisor = yield* makeExternalProviderProcessSupervisor();
      const child = yield* supervisor.processes.spawn({
        command: process.env.T3_PI_SMOKE_BINARY!,
        cwd: directory,
        purpose: { kind: "session", threadId },
        args: piLaunchArgs(
          ["--no-skills", "--no-prompt-templates", "--no-themes", "--extension", observerPath],
          path.join(directory, "sessions"),
          ["--extension", wrapperPath],
        ),
        environment: { PI_CODING_AGENT_DIR: path.join(directory, "agent") },
        protectedEnvironment: {
          T3_PI_MCP_ENDPOINT: credential.config.endpoint,
          T3_PI_MCP_AUTHORIZATION: credential.config.authorizationHeader,
        },
      });
      const lines = yield* Queue.unbounded<string>();
      yield* child.stdout.pipe(
        Stream.decodeText(),
        Stream.splitLines,
        Stream.runForEach((line) => Queue.offer(lines, line)),
        Effect.forkScoped,
      );
      const readUntil = (prefix: string) =>
        Effect.gen(function* () {
          for (;;) {
            const line = yield* Queue.take(lines);
            if (line.startsWith(prefix)) return line;
          }
        });
      const smoke = Effect.gen(function* () {
        yield* child.write(
          new TextEncoder().encode(
            '{"id":"smoke","type":"prompt","message":"/t3-preview-smoke"}\n',
          ),
        );
        const line = yield* readUntil("T3_SMOKE");
        expect(line.includes(credential.config.authorizationHeader.slice(7))).toBe(false);
        expect(line).toMatch(/^T3_SMOKE /);
        const result = decodeResult(line.slice("T3_SMOKE ".length));
        expect(result.names).toContain("preview_status");
        expect(result.names.every((name) => name.startsWith("preview_"))).toBe(true);
        expect(result.status).toContain("Isolated fixture");
        expect(result.opened).toContain("Isolated fixture");
        expect(result.image).toBe(true);
      });
      yield* smoke;
      const priorPath = path.join(directory, "prior.jsonl");
      yield* fs.writeFileString(
        priorPath,
        encodeJson({
          type: "session",
          version: 3,
          id: "19104a2a-800e-494d-ab03-0ff21a163cc0",
          timestamp: "2026-09-27T00:00:00.000Z",
          cwd: directory,
        }) + "\n",
      );
      yield* child.write(
        new TextEncoder().encode(
          encodeJson({ id: "resume", type: "switch_session", sessionPath: priorPath }) + "\n",
        ),
      );
      const resumed = yield* readUntil('{"id":"resume"');
      expect(resumed).toContain('"success":true');
      yield* smoke;
      const observations = yield* fs.readFileString(observationsPath);
      expect(observations.split("\n").filter(Boolean)).toHaveLength(4);
      expect(observations).not.toContain('"exposed":true');
      expect(operations).toEqual(["status", "open", "snapshot", "status", "open", "snapshot"]);
      yield* child.close;
      yield* registry.revokeThread(threadId);
    }).pipe(Effect.scoped, Effect.provide(layer)),
  { timeout: 30_000, skip: !process.env.T3_PI_SMOKE_BINARY },
);

for (const failureStage of ["startup", "resume", "healthy-resume"] as const) {
  it.live(
    `Pi adapter checks bridge health during ${failureStage}`,
    () =>
      Effect.gen(function* () {
        yield* HttpRouter.serve(serverLayer, { disableListenLog: true, disableLogger: true }).pipe(
          Layer.build,
        );
        const registry = yield* McpSessionRegistry.McpSessionRegistry;
        const instanceId = ProviderInstanceId.make("piRpc_failed_bootstrap");
        const issued = yield* registry.issue({
          threadId,
          providerInstanceId: instanceId,
          capabilities: new Set(["preview"]),
        });
        if (failureStage === "startup") yield* registry.revokeThread(threadId);
        const fs = yield* FileSystem.FileSystem;
        const path = yield* Path.Path;
        const directory = yield* fs.makeTempDirectoryScoped({ prefix: "pi-failed-bootstrap-" });
        const { host } = yield* makeExternalProviderAdapterHostV2({
          packageId: ProviderAdapterPackageId.make("pi-rpc"),
          instanceId,
        });
        const priorId = "b0b2f32b-5090-465e-9d20-8f7ef51e5967";
        const storage = yield* host.storage.prepareSession(threadId);
        const priorPath = path.join(storage.sessionDirectory, "prior.jsonl");
        yield* fs.writeFileString(
          priorPath,
          encodeJson({
            type: "session",
            version: 3,
            id: priorId,
            timestamp: "2026-09-27T00:00:00.000Z",
            cwd: directory,
          }) + "\n",
        );
        const config = { binaryPath: process.env.T3_PI_SMOKE_BINARY!, args: [] };
        const output: string[] = [];
        let resumeAttempted = false;
        const instance = yield* makePiAdapter(
          config,
          {
            instanceId,
            displayName: "Pi test",
            accentColor: undefined,
            enabled: true,
            config,
            environment: { PI_CODING_AGENT_DIR: path.join(directory, "agent") },
          },
          {
            ...host,
            storage: {
              ...host.storage,
              validateSessionFile: (input) =>
                host.storage.validateSessionFile(input).pipe(
                  Effect.tap(() => {
                    if (input.mustExist) resumeAttempted = true;
                    return input.mustExist && failureStage === "resume"
                      ? registry.revokeThread(threadId)
                      : Effect.void;
                  }),
                ),
            },
            mcp: { readSession: () => Effect.succeed(issued.config) },
            processes: {
              spawn: (input) => {
                expect(
                  input.args?.some((arg) =>
                    arg.includes(issued.config.authorizationHeader.slice(7)),
                  ),
                ).toBe(false);
                return host.processes.spawn(input).pipe(
                  Effect.map((child) => ({
                    ...child,
                    stdout: child.stdout.pipe(
                      Stream.tap((chunk) =>
                        Effect.sync(() => {
                          output.push(new TextDecoder().decode(chunk));
                        }),
                      ),
                    ),
                    stderr: child.stderr.pipe(
                      Stream.tap((chunk) =>
                        Effect.sync(() => {
                          output.push(new TextDecoder().decode(chunk));
                        }),
                      ),
                    ),
                  })),
                );
              },
            },
          },
        );
        const result = yield* instance.adapter
          .startSession({
            threadId,
            runtimeMode: "full-access",
            cwd: directory,
            ...(failureStage !== "startup"
              ? { resumeCursor: { sessionFile: priorPath, sessionId: priorId } }
              : {}),
          })
          .pipe(Effect.result);
        expect(resumeAttempted).toBe(failureStage !== "startup");
        expect(result._tag).toBe(failureStage === "healthy-resume" ? "Success" : "Failure");
        if (result._tag === "Failure") expect(result.failure.detail).toContain("preview bridge");
        else
          expect(result.success.resumeCursor).toEqual({
            sessionFile: yield* fs.realPath(priorPath),
            sessionId: priorId,
          });
        expect((yield* instance.adapter.listSessions()).length).toBe(
          failureStage === "healthy-resume" ? 1 : 0,
        );
        expect(output.join("").includes(issued.config.authorizationHeader)).toBe(false);
        expect(output.join("").includes(issued.config.authorizationHeader.slice(7))).toBe(false);
      }).pipe(Effect.scoped, Effect.provide(layer)),
    { timeout: 30_000, skip: !process.env.T3_PI_SMOKE_BINARY },
  );
}
