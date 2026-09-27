import type { ThreadId } from "@t3tools/contracts";
import type { ProviderAdapterHostV2 } from "@t3tools/provider-adapter";
import * as Effect from "effect/Effect";

export const PI_PREVIEW_HEALTH_COMMAND = "t3-preview-health";

export const PI_PREVIEW_EMPTY_ENVIRONMENT = {
  T3_PI_MCP_ENDPOINT: undefined,
  T3_PI_MCP_AUTHORIZATION: undefined,
};

// Self-contained source is materialized by the host, so bundled server releases
// do not depend on a source-tree path or on Pi resolving T3's dependencies.
export const PI_PREVIEW_EXTENSION_SOURCE = String.raw`
export default async function t3Preview(pi) {
  // Pi recreates factories on resume/new/fork/reload. Keep the handoff in
  // process memory, never re-expose it to later factories through process.env.
  // This is not a sandbox: same-process extensions are trusted code.
  const credentialKey = Symbol.for("t3.pi.preview.credential");
  const previous = globalThis[credentialKey];
  const endpoint = process.env.T3_PI_MCP_ENDPOINT || previous?.endpoint;
  const authorization = process.env.T3_PI_MCP_AUTHORIZATION || previous?.authorization;
  delete process.env.T3_PI_MCP_ENDPOINT;
  delete process.env.T3_PI_MCP_AUTHORIZATION;
  if (!endpoint || !authorization) return;
  const credential = { endpoint, authorization };
  Object.defineProperty(globalThis, credentialKey, { value: credential, configurable: true });
  const forgetCredential = () => {
    if (globalThis[credentialKey] === credential) delete globalThis[credentialKey];
  };

  let sessionId;
  let sequence = 0;
  const lifetime = new AbortController();
  const headers = () => ({
    authorization,
    "content-type": "application/json",
    accept: "application/json, text/event-stream",
    "mcp-protocol-version": "2025-06-18",
    ...(sessionId ? { "mcp-session-id": sessionId } : {}),
  });
  const post = (message, signal) => fetch(endpoint, {
    method: "POST", headers: headers(), body: JSON.stringify(message), signal,
  });
  const notify = async (method, params) => {
    const response = await post({ jsonrpc: "2.0", method, params }, AbortSignal.timeout(5000));
    await response.body?.cancel();
    if (!response.ok) throw new Error("T3 preview notification failed.");
  };
  const readReply = async (response, id) => {
    if (!response.headers.get("content-type")?.includes("text/event-stream")) {
      return await response.json();
    }
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    try {
      while (true) {
        const chunk = await reader.read();
        buffer += decoder.decode(chunk.value, { stream: !chunk.done });
        let boundary;
        while ((boundary = /\r?\n\r?\n/.exec(buffer)) !== null) {
          const frame = buffer.slice(0, boundary.index);
          buffer = buffer.slice(boundary.index + boundary[0].length);
          const data = frame.split(/\r?\n/).filter(line => line.startsWith("data:"))
            .map(line => line.slice(5).trimStart()).join("\n");
          if (!data) continue;
          const message = JSON.parse(data);
          if (message.id === id) return message;
        }
        if (chunk.done) throw new Error("Missing T3 preview response.");
      }
    } finally {
      await reader.cancel();
      reader.releaseLock();
    }
  };
  const request = async (method, params, signal) => {
    const id = ++sequence;
    const combined = AbortSignal.any([lifetime.signal, AbortSignal.timeout(120000), ...(signal ? [signal] : [])]);
    combined.throwIfAborted();
    const cancel = () => { void notify("notifications/cancelled", { requestId: id }).catch(() => {}); };
    combined.addEventListener("abort", cancel, { once: true });
    try {
      const response = await post({ jsonrpc: "2.0", id, method, params }, combined);
      if (!response.ok) {
        await response.body?.cancel();
        throw new Error("T3 preview request rejected.");
      }
      sessionId = response.headers.get("mcp-session-id") || sessionId;
      const message = await readReply(response, id);
      if (message.id !== id || message.error || !message.result) throw new Error("Invalid T3 preview response.");
      return message.result;
    } catch {
      // Never surface fetch errors, response bodies, or headers containing credentials.
      throw new Error(combined.aborted ? "T3 preview request cancelled." : "T3 preview request failed. Restart the provider session if access was revoked.");
    } finally {
      combined.removeEventListener("abort", cancel);
    }
  };
  const shutdown = async () => {
    lifetime.abort();
    if (sessionId) {
      try {
        const response = await fetch(endpoint, { method: "DELETE", headers: headers(), signal: AbortSignal.timeout(5000) });
        await response.body?.cancel();
      } catch { /* Process exit also closes the transport. */ }
    }
  };
  pi.on("session_shutdown", async event => {
    await shutdown();
    if (!["resume", "reload", "new", "fork"].includes(event?.reason)) forgetCredential();
  });

  try {
    const bootstrap = AbortSignal.timeout(10000);
    await request("initialize", {
      protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "t3-pi-preview", version: "1.0.0" },
    }, bootstrap);
    await notify("notifications/initialized", {});
    let cursor;
    const tools = [];
    do {
      const page = await request("tools/list", cursor ? { cursor } : {}, bootstrap);
      if (!Array.isArray(page.tools)) throw new Error("Invalid T3 preview catalog.");
      tools.push(...page.tools.filter(tool => typeof tool.name === "string" && tool.name.startsWith("preview_")));
      cursor = page.nextCursor;
    } while (cursor);
    if (!["preview_status", "preview_open", "preview_snapshot"].every(name => tools.some(tool => tool.name === name))) {
      throw new Error("Incomplete T3 preview catalog.");
    }
    for (const tool of tools) {
      pi.registerTool({
        name: tool.name, label: tool.name, description: tool.description || tool.name,
        parameters: tool.inputSchema,
        async execute(_id, params, signal) {
          const result = await request("tools/call", { name: tool.name, arguments: params }, signal);
          const redact = text => text.split(authorization).join("[redacted]").split(authorization.replace(/^Bearer /, "")).join("[redacted]");
          const content = (result.content || []).filter(block => block.type === "text" || block.type === "image")
            .map(block => block.type === "text" ? { type: "text", text: redact(block.text) } : { type: "image", data: block.data, mimeType: block.mimeType });
          if (result.isError) {
            throw new Error(content.filter(block => block.type === "text").map(block => block.text).join("\n") || "T3 preview tool failed.");
          }
          return { content, details: {} };
        },
      });
    }
    // get_commands is Pi's RPC acknowledgement surface. Register this only
    // after the required tools, so a swallowed factory error fails T3 startup.
    pi.registerCommand("${PI_PREVIEW_HEALTH_COMMAND}", {
      description: "Check T3 browser tool registration",
      handler: async (_args, ctx) => ctx.ui.notify("T3 browser tools are registered. Use preview_status to check browser availability.", "info"),
    });
    if (tools.length) pi.on("before_agent_start", event => ({
      systemPrompt: event.systemPrompt + "\nT3 Code collaborative browser: use preview_status first. If no automation-capable preview is attached, call preview_open. Use preview_snapshot locators and the preview_* tools for the browser shared with the user. Tool registration does not mean a preview host is ready. Only use another browser when explicitly requested or preview_open reports unsupported/unavailable.",
    }));
  } catch {
    forgetCredential();
    await shutdown();
    throw new Error("T3 preview bridge initialization failed. Restart the provider session to retry.");
  }
}
`;

export const preparePiPreviewExtension = Effect.fn("preparePiPreviewExtension")(function* (
  host: ProviderAdapterHostV2,
  threadId: ThreadId,
) {
  const session = host.mcp ? yield* host.mcp.readSession(threadId) : undefined;
  if (!session?.capabilities.has("preview")) {
    return { required: false, args: [], protectedEnvironment: PI_PREVIEW_EMPTY_ENVIRONMENT };
  }
  const path = yield* host.storage.materializeArtifact({
    key: "t3-preview",
    fileName: "t3-preview.mjs",
    content: PI_PREVIEW_EXTENSION_SOURCE,
  });
  return {
    required: true,
    args: ["--extension", path],
    protectedEnvironment: {
      T3_PI_MCP_ENDPOINT: session.endpoint,
      T3_PI_MCP_AUTHORIZATION: session.authorizationHeader,
    },
  };
});
