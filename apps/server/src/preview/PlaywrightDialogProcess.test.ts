import * as NodeChildProcess from "node:child_process";
import * as NodeURL from "node:url";
import { expect, it } from "vite-plus/test";

const probePath = NodeURL.fileURLToPath(
  new URL("./fixtures/playwright-dialog-process.mjs", import.meta.url),
);

it.each([
  {
    scenario: "close-race",
    accept: false,
    behavior: "survives automatic dismissal racing another CDP client",
  },
  {
    scenario: "beforeunload-race",
    accept: true,
    behavior: "survives automatic beforeunload acceptance racing another CDP client",
  },
  { scenario: "alert", accept: false, behavior: "automatically dismisses alerts" },
  { scenario: "confirm", accept: false, behavior: "automatically dismisses confirmations" },
  { scenario: "prompt", accept: false, behavior: "automatically dismisses prompts" },
  {
    scenario: "beforeunload",
    accept: true,
    behavior: "automatically accepts beforeunload dialogs",
  },
  {
    scenario: "handled",
    accept: false,
    behavior: "leaves handled dialogs to their owner and preserves explicit API errors",
  },
])("$behavior", ({ scenario, accept }) => {
  const child = NodeChildProcess.spawnSync(
    process.execPath,
    ["--unhandled-rejections=strict", probePath, scenario],
    { encoding: "utf8", timeout: 10_000 },
  );
  expect(child.error).toBeUndefined();
  expect(child.signal).toBeNull();
  expect(child.status, child.stderr).toBe(0);
  expect(JSON.parse(child.stdout)).toEqual({ scenario, calls: [accept], survived: true });
});
