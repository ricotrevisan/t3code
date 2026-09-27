// @effect-diagnostics nodeBuiltinImport:off
import * as NodeHttp from "node:http";
import * as NodeVM from "node:vm";
import { describe, expect, it } from "@effect/vitest";
import * as Effect from "effect/Effect";
import { ThreadId, ProviderInstanceId } from "@t3tools/contracts";
import {
  ProviderAdapterHostProcessError,
  type ProviderAdapterHostV2,
  type ProviderAdapterProcessSpawnV1,
} from "@t3tools/provider-adapter";
import { makePiAdapter } from "./PiAdapter.ts";
import { PI_PREVIEW_EXTENSION_SOURCE, preparePiPreviewExtension } from "./piPreviewExtension.ts";

type Block = { type: string; text?: string; data?: string; mimeType?: string };
type Tool = {
  name: string;
  parameters: unknown;
  execute: (id: string, params: unknown, signal?: AbortSignal) => Promise<{ content: Block[] }>;
};

async function loadBridge(endpoint: string, authorization: string) {
  const tools = new Map<string, Tool>();
  const handlers = new Map<string, (event?: unknown) => Promise<unknown>>();
  const env = { T3_PI_MCP_ENDPOINT: endpoint, T3_PI_MCP_AUTHORIZATION: authorization };
  const factory = NodeVM.runInNewContext(
    PI_PREVIEW_EXTENSION_SOURCE.replace("export default ", "") + "\nt3Preview;",
    {
      process: { env },
      fetch,
      AbortController,
      AbortSignal,
      TextDecoder,
    },
  );
  await factory({
    registerTool: (tool: Tool) => tools.set(tool.name, tool),
    on: (name: string, handler: (event?: unknown) => Promise<unknown>) =>
      handlers.set(name, handler),
  });
  return { tools, handlers, env };
}

async function serverFixture() {
  const calls: Array<{
    auth: string | undefined;
    method: string;
    params: Record<string, unknown>;
  }> = [];
  const revoked = new Set<string>();
  const pending = Promise.withResolvers<void>();
  const cancelled = Promise.withResolvers<void>();
  const server = NodeHttp.createServer(async (req, res) => {
    if (revoked.has(req.headers.authorization ?? "")) {
      res.writeHead(401).end();
      return;
    }
    if (req.method === "DELETE") {
      res.writeHead(204).end();
      return;
    }
    let body = "";
    for await (const chunk of req) body += chunk;
    const message = JSON.parse(body);
    calls.push({ auth: req.headers.authorization, method: message.method, params: message.params });
    if (message.method === "notifications/cancelled") cancelled.resolve();
    if (!message.id) {
      res.writeHead(202).end();
      return;
    }
    if (message.params?.arguments?.wait) {
      pending.resolve();
      return;
    }
    const result =
      message.method === "initialize"
        ? { protocolVersion: "2025-06-18" }
        : message.method === "tools/list"
          ? {
              tools: ["preview_status", "preview_open", "preview_snapshot", "device_status"].map(
                (name) => ({ name, inputSchema: { type: "object", properties: {} } }),
              ),
            }
          : {
              content: [
                {
                  type: "text",
                  text: req.headers.authorization === "Bearer first" ? "thread-one" : "thread-two",
                },
                { type: "image", data: "cG5n", mimeType: "image/png" },
              ],
              isError: message.params?.arguments?.fail === true,
            };
    res.writeHead(200, {
      "content-type": "text/event-stream",
      "mcp-session-id": "transport-session",
    });
    res.end(
      `event: message\ndata: ${JSON.stringify({ jsonrpc: "2.0", id: message.id, result })}\n\n`,
    );
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  if (!address || typeof address === "string") throw new Error("Missing port");
  return {
    endpoint: `http://127.0.0.1:${address.port}/mcp`,
    calls,
    revoked,
    pending: pending.promise,
    cancelled: cancelled.promise,
    close: () => {
      server.closeAllConnections();
      server.close();
    },
  };
}

describe("Pi preview bridge", () => {
  it("forwards only preview tools, preserves text/images, isolates credentials and fails after revocation", async () => {
    const server = await serverFixture();
    try {
      const first = await loadBridge(server.endpoint, "Bearer first");
      const second = await loadBridge(server.endpoint, "Bearer second");
      expect([...first.tools.keys()]).toEqual([
        "preview_status",
        "preview_open",
        "preview_snapshot",
      ]);
      expect(first.env).toEqual({});
      expect(await first.tools.get("preview_snapshot")!.execute("1", {})).toEqual({
        content: [
          { type: "text", text: "thread-one" },
          { type: "image", data: "cG5n", mimeType: "image/png" },
        ],
        details: {},
      });
      expect((await second.tools.get("preview_status")!.execute("2", {})).content[0]?.text).toBe(
        "thread-two",
      );
      await expect(first.tools.get("preview_open")!.execute("3", { fail: true })).rejects.toThrow(
        "thread-one",
      );
      server.revoked.add("Bearer first");
      await expect(first.tools.get("preview_status")!.execute("4", {})).rejects.toThrow(
        "T3 preview request failed",
      );
      expect((await second.tools.get("preview_status")!.execute("5", {})).content[0]?.text).toBe(
        "thread-two",
      );
      await second.handlers.get("session_shutdown")!();
      await expect(second.tools.get("preview_status")!.execute("6", {})).rejects.toThrow();
    } finally {
      server.close();
    }
  });

  it("aborts in-flight HTTP and sends MCP cancellation", async () => {
    const server = await serverFixture();
    try {
      const bridge = await loadBridge(server.endpoint, "Bearer first");
      const controller = new AbortController();
      const call = bridge.tools
        .get("preview_open")!
        .execute("1", { wait: true }, controller.signal);
      const rejected = expect(call).rejects.toThrow("cancelled");
      await server.pending;
      controller.abort();
      await rejected;
      await server.cancelled;
      expect(server.calls.at(-1)?.params.requestId).toBe(3);
      await bridge.handlers.get("session_shutdown")!();
    } finally {
      server.close();
    }
  });

  it("closes the old transport on reload and hands credentials to the replacement factory", async () => {
    const server = await serverFixture();
    try {
      const bridge = await loadBridge(server.endpoint, "Bearer first");
      await bridge.handlers.get("session_shutdown")!({ reason: "reload" });
      expect(bridge.env).toEqual({
        T3_PI_MCP_ENDPOINT: server.endpoint,
        T3_PI_MCP_AUTHORIZATION: "Bearer first",
      });
      await expect(bridge.tools.get("preview_status")!.execute("old", {})).rejects.toThrow();
      const replacement = await loadBridge(
        bridge.env.T3_PI_MCP_ENDPOINT,
        bridge.env.T3_PI_MCP_AUTHORIZATION,
      );
      expect(replacement.env).toEqual({});
      expect(
        (await replacement.tools.get("preview_status")!.execute("new", {})).content[0]?.text,
      ).toBe("thread-one");
      await replacement.handlers.get("session_shutdown")!();
    } finally {
      server.close();
    }
  });

  it.effect(
    "keeps older hosts and denied sessions credential-free and materializes only trusted source",
    () =>
      Effect.gen(function* () {
        const artifacts: unknown[] = [];
        const host: ProviderAdapterHostV2 = {
          protocolVersion: 2,
          processes: { spawn: () => Effect.die("unused") },
          workspaces: { resolveCwd: () => Effect.succeed("/tmp") },
          attachments: { read: () => Effect.die("unused") },
          storage: {
            prepareSession: () => Effect.die("unused"),
            validateSessionFile: () => Effect.die("unused"),
            materializeArtifact: (input) => {
              artifacts.push(input);
              return Effect.succeed("/trusted/bridge.mjs");
            },
          },
        };
        const thread = ThreadId.make("one");
        expect((yield* preparePiPreviewExtension(host, thread)).args).toEqual([]);
        const mcp = {
          readSession: (id: ThreadId) =>
            Effect.succeed({
              endpoint: "http://localhost/mcp",
              authorizationHeader: `Bearer ${id}`,
              capabilities: new Set(id === thread ? ["preview"] : []),
            }),
        };
        const allowed = yield* preparePiPreviewExtension({ ...host, mcp }, thread);
        expect(allowed.args).toEqual(["--extension", "/trusted/bridge.mjs"]);
        expect(allowed.protectedEnvironment.T3_PI_MCP_AUTHORIZATION).toBe("Bearer one");
        const denied = yield* preparePiPreviewExtension({ ...host, mcp }, ThreadId.make("denied"));
        expect(denied.args).toEqual([]);
        expect(denied.protectedEnvironment.T3_PI_MCP_AUTHORIZATION).toBeUndefined();
        expect(artifacts).toHaveLength(1);
        expect(artifacts[0]).toEqual({
          key: "t3-preview",
          fileName: "t3-preview.mjs",
          content: PI_PREVIEW_EXTENSION_SOURCE,
        });
      }),
  );
  it.effect(
    "launches a bridge only for eligible sessions, refreshes credentials, and never reads MCP for probes",
    () =>
      Effect.gen(function* () {
        const spawns: ProviderAdapterProcessSpawnV1[] = [];
        const reads: ThreadId[] = [];
        let credential = "Bearer initial";
        const host: ProviderAdapterHostV2 = {
          protocolVersion: 2,
          processes: {
            spawn: (input) => {
              spawns.push(input);
              return Effect.fail(
                new ProviderAdapterHostProcessError({
                  operation: "spawn",
                  detail: "fixture spawn stopped",
                }),
              );
            },
          },
          workspaces: { resolveCwd: () => Effect.succeed("/tmp") },
          storage: {
            prepareSession: (id) =>
              Effect.succeed({ sessionDirectory: `/tmp/${id}`, sharedDirectory: "/tmp" }),
            validateSessionFile: () => Effect.die("unused"),
            materializeArtifact: () => Effect.succeed("/trusted/bridge.mjs"),
          },
          attachments: { read: () => Effect.die("unused") },
          mcp: {
            readSession: (id) => {
              reads.push(id);
              return Effect.succeed({
                endpoint: "http://localhost/mcp",
                authorizationHeader: credential,
                capabilities: new Set(id === ThreadId.make("allowed") ? ["preview"] : []),
              });
            },
          },
        };
        const config = { binaryPath: "pi", args: ["--extension", "/user/extension.ts"] };
        const instance = yield* makePiAdapter(
          config,
          {
            instanceId: ProviderInstanceId.make("pi-test"),
            displayName: "Pi test",
            accentColor: undefined,
            enabled: true,
            config,
            environment: { T3_PI_MCP_AUTHORIZATION: "untrusted-instance" },
          },
          host,
        );
        const start = (id: string) =>
          instance.adapter
            .startSession({ threadId: ThreadId.make(id), runtimeMode: "full-access" })
            .pipe(Effect.result);
        yield* start("allowed");
        credential = "Bearer recovered";
        yield* start("allowed");
        yield* start("denied");
        yield* instance.snapshot.getSnapshot;
        expect(reads).toEqual(["allowed", "allowed", "denied"]);
        expect(spawns[0]?.args).toContain("/user/extension.ts");
        expect(spawns[0]?.args).toContain("/trusted/bridge.mjs");
        expect(spawns[0]?.protectedEnvironment?.T3_PI_MCP_AUTHORIZATION).toBe("Bearer initial");
        expect(spawns[1]?.protectedEnvironment?.T3_PI_MCP_AUTHORIZATION).toBe("Bearer recovered");
        for (const spawn of spawns.slice(2)) {
          expect(spawn.args).not.toContain("/trusted/bridge.mjs");
          expect(spawn.protectedEnvironment?.T3_PI_MCP_AUTHORIZATION).toBeUndefined();
        }
        expect(spawns[3]?.purpose.kind).toBe("probe");
        expect(spawns.every((spawn) => !spawn.args?.some((arg) => arg.includes("Bearer")))).toBe(
          true,
        );
      }).pipe(Effect.scoped),
  );
});
