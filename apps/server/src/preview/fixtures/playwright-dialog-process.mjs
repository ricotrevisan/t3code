import * as NodeAssert from "node:assert/strict";
import * as NodeFS from "node:fs";
import * as NodeModule from "node:module";
import * as NodePath from "node:path";
import * as NodeVM from "node:vm";

// Playwright does not export its server-side dialog classes. Execute the real
// installed bundle unchanged, exposing only the constructors for this probe.
// No browser, CDP connection, source substitution, or global rejection handler.
const require = NodeModule.createRequire(import.meta.url);
const bundlePath = NodePath.join(
  NodePath.dirname(require.resolve("playwright-core/package.json")),
  "lib/coreBundle.js",
);
const load = NodeVM.compileFunction(
  `${NodeFS.readFileSync(bundlePath, "utf8")}
return { Dialog, DialogManager, ProtocolError };`,
  ["exports", "require", "module", "__filename", "__dirname"],
  { filename: bundlePath },
);
const { Dialog, DialogManager, ProtocolError } = load(
  {},
  NodeModule.createRequire(bundlePath),
  { exports: {} },
  bundlePath,
  NodePath.dirname(bundlePath),
);
const scenario = process.argv[2];
NodeAssert.ok(
  [
    "close-race",
    "beforeunload-race",
    "alert",
    "confirm",
    "prompt",
    "beforeunload",
    "handled",
  ].includes(scenario),
  `Unknown dialog probe scenario: ${scenario}`,
);
/** @type {boolean[]} */
const calls = [];
const manager = new DialogManager({ onDialog() {} });
const page = {
  attribution: {},
  instrumentation: {},
  browserContext: { dialogManager: manager },
  frameManager: { frames: () => [] },
};
const failure = new ProtocolError("error", "Page.handleJavaScriptDialog");
failure.setMessage("No dialog is showing");
const dialog = new Dialog(
  page,
  scenario === "beforeunload-race"
    ? "beforeunload"
    : scenario === "close-race" || scenario === "handled"
      ? "alert"
      : scenario,
  "regression probe",
  /** @param {boolean} accept */
  async (accept) => {
    calls.push(accept);
    if (scenario === "close-race" || scenario === "beforeunload-race" || scenario === "handled")
      throw failure;
  },
  "",
);
if (scenario === "handled") manager.addDialogHandler(() => true);
manager.dialogDidOpen(dialog);
if (scenario === "handled") {
  NodeAssert.deepEqual(calls, []);
  NodeAssert.equal(manager.hasOpenDialogsForPage(page), true);
  // Explicit API calls still surface errors to their caller; only the
  // unowned, best-effort close is contained by the dependency patch.
  await NodeAssert.rejects(
    dialog.dismiss({ race: /** @param {Promise<void>} promise */ (promise) => promise }),
    failure,
  );
}
// Cross Node's unhandled-rejection checkpoint, not a timed sleep. An unhandled
// rejection kills this subprocess before it can report successful survival.
setImmediate(() => {
  NodeAssert.deepEqual(calls, [scenario === "beforeunload" || scenario === "beforeunload-race"]);
  NodeAssert.equal(manager.hasOpenDialogsForPage(page), false);
  console.log(JSON.stringify({ scenario, calls, survived: true }));
});
