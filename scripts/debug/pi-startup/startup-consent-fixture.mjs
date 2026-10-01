export default function startupConsentFixture(pi) {
  pi.registerCommand("t3-preview-health", { description: "Synthetic bridge health acknowledgement", handler: async () => {} });
  pi.on("session_start", async (_event, ctx) => {
    const keepAlive = setInterval(() => {}, 1000);
    try {
      await ctx.ui.confirm("Fixture project MCP approval", "Synthetic approval; no external service");
    } finally {
      clearInterval(keepAlive);
    }
  });
}
