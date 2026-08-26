/// <reference lib="dom" />
// @effect-diagnostics nodeBuiltinImport:off globalDate:off globalTimers:off unknownInEffectCatch:off anyUnknownInErrorContext:off -- The lab host owns the Chromium subprocess, its CDP socket, and raw timers at a browser boundary.
/**
 * Headless PreviewAutomationHost for `t3 serve`.
 *
 * Desktop Electron is the usual host. On this lab VPS nothing registers, so
 * preview_* fails with PreviewAutomationNoAvailableHostError. This layer
 * speaks the same broker connect/respond protocol and drives the persistent
 * system Chromium profile via CDP. Agents keep using preview_*.
 */

import * as NodeChildProcess from "node:child_process";
import * as NodeFs from "node:fs";
import * as NodeHttp from "node:http";
import * as NodeOs from "node:os";
import * as NodePath from "node:path";

import {
  FILL_PREVIEW_VIEWPORT,
  PREVIEW_AUTOMATION_OPERATIONS,
  type PreviewAutomationClickInput,
  type PreviewAutomationEvaluateInput,
  type PreviewAutomationHost,
  type PreviewAutomationNavigateInput,
  type PreviewAutomationOpenInput,
  type PreviewAutomationPressInput,
  type PreviewAutomationRequest,
  type PreviewAutomationResizeInput,
  type PreviewAutomationResponse,
  type PreviewAutomationScrollInput,
  type PreviewAutomationSetColorSchemeInput,
  type PreviewAutomationSnapshot,
  type PreviewAutomationStatus,
  type PreviewAutomationTypeInput,
  type PreviewAutomationWaitForInput,
  type PreviewTabId,
  type PreviewViewportSetting,
} from "@t3tools/contracts";
import { newPreviewTabId, normalizePreviewUrl } from "@t3tools/shared/preview";
import { resolvePreviewViewport } from "@t3tools/shared/previewViewport";
import * as Effect from "effect/Effect";
import * as Layer from "effect/Layer";
import * as Result from "effect/Result";
import * as Stream from "effect/Stream";
import type { Browser, BrowserContext, Page } from "playwright-core";
import { chromium } from "playwright-core";

import * as ServerEnvironment from "../environment/ServerEnvironment.ts";
import * as PreviewAutomationBroker from "../mcp/PreviewAutomationBroker.ts";

export const LAB_CHROMIUM_CLIENT_ID = "lab-chromium";
export const LAB_CDP_HOST = "127.0.0.1";
export const LAB_CDP_PORT = 9222;
const DEFAULT_VIEWPORT = { width: 1280, height: 720 } as const;
const MAX_VISIBLE_TEXT_LENGTH = 20_000;
const MAX_INTERACTIVE_ELEMENTS = 200;
const MAX_SCREENSHOT_WIDTH = 1280;

export const defaultLabCdpOrigin = (): string =>
  `http://${LAB_CDP_HOST}:${Number(process.env.T3_BROWSER_DEBUG_PORT ?? LAB_CDP_PORT)}`;

export const defaultLabBrowserCommand = (): string => {
  const fromEnv = process.env.T3_BROWSER_BIN;
  if (fromEnv && fromEnv.length > 0) return fromEnv;
  const local = NodePath.join(NodeOs.homedir(), ".local/bin/t3-browser");
  if (NodeFs.existsSync(local)) return local;
  return "t3-browser";
};

export const resolveLabNavigationUrl = (input: {
  readonly url?: string | undefined;
  readonly target?: PreviewAutomationNavigateInput["target"];
}): string => {
  if (input.target?.kind === "environment-port") {
    const protocol = input.target.protocol ?? "http";
    const path = input.target.path ?? "/";
    const suffix = path.startsWith("/") ? path : `/${path}`;
    return `${protocol}://127.0.0.1:${input.target.port}${suffix}`;
  }
  if (input.url === undefined || input.url.trim().length === 0) {
    throw new Error("Provide exactly one of url or target.");
  }
  return normalizePreviewUrl(input.url);
};

class LabHostOperationError extends Error {
  readonly responseTag: string;
  readonly detail?: unknown;
  constructor(responseTag: string, message: string, detail?: unknown) {
    super(message);
    this.responseTag = responseTag;
    this.detail = detail;
  }
}

const tabNotFound = (operation: string, tabId?: string) =>
  new LabHostOperationError(
    "PreviewAutomationTabNotFoundError",
    tabId
      ? `Preview tab ${tabId} was not found for ${operation}.`
      : `No active preview tab was found for ${operation}.`,
    { tabId: tabId ?? null, operation },
  );

const recordingUnavailable = (operation: string) =>
  new LabHostOperationError(
    "PreviewAutomationRemoteUnavailableError",
    `Preview automation ${operation} is unavailable on the lab Chromium host.`,
    { operation },
  );

export const serializeLabHostError = (
  error: unknown,
  request: PreviewAutomationRequest,
): NonNullable<PreviewAutomationResponse["error"]> => {
  if (error instanceof LabHostOperationError) {
    return {
      _tag: error.responseTag,
      message: error.message,
      ...(error.detail === undefined ? {} : { detail: error.detail }),
    };
  }
  const message = error instanceof Error ? error.message : String(error);
  return {
    _tag: "PreviewAutomationExecutionError",
    message: `Preview automation ${request.operation} failed on client ${LAB_CHROMIUM_CLIENT_ID}.`,
    detail: { causeMessage: message, operation: request.operation },
  };
};

const waitForCdp = async (origin: string, timeoutMs: number): Promise<void> => {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const ok = await new Promise<boolean>((resolve) => {
      const req = NodeHttp.get(`${origin}/json/version`, (res) => {
        res.resume();
        resolve((res.statusCode ?? 500) < 300);
      });
      req.on("error", () => resolve(false));
      req.setTimeout(500, () => {
        req.destroy();
        resolve(false);
      });
    });
    if (ok) return;
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  throw new Error(`Lab Chromium CDP did not become ready at ${origin}`);
};

export interface LabBrowserTab {
  readonly tabId: PreviewTabId;
  readonly page: Page;
}

export interface LabBrowserRuntime {
  readonly browser: Browser;
  readonly context: BrowserContext;
  readonly spawned: NodeChildProcess.ChildProcess | null;
  tabs: Map<string, Page>;
  threadTabs: Map<string, PreviewTabId>;
  viewportSetting: PreviewViewportSetting;
}

const bindPage = (
  runtime: LabBrowserRuntime,
  page: Page,
  tabId: PreviewTabId = newPreviewTabId() as PreviewTabId,
): LabBrowserTab => {
  runtime.tabs.set(tabId, page);
  page.on("close", () => {
    runtime.tabs.delete(tabId);
  });
  return { tabId, page };
};

export const attachLabBrowserRuntime = async (options: {
  readonly origin: string;
  readonly spawned: NodeChildProcess.ChildProcess | null;
}): Promise<LabBrowserRuntime> => {
  const browser = await chromium.connectOverCDP(options.origin);
  const context =
    browser.contexts()[0] ?? (await browser.newContext({ viewport: DEFAULT_VIEWPORT }));
  const runtime: LabBrowserRuntime = {
    browser,
    context,
    spawned: options.spawned,
    tabs: new Map(),
    threadTabs: new Map(),
    viewportSetting: FILL_PREVIEW_VIEWPORT,
  };
  const pages = context.pages();
  if (pages.length === 0) {
    bindPage(runtime, await context.newPage());
  } else {
    for (const page of pages) bindPage(runtime, page);
  }
  return runtime;
};

export const ensureLabChromium = async (options?: {
  readonly origin?: string;
  readonly command?: string;
}): Promise<{ origin: string; spawned: NodeChildProcess.ChildProcess | null }> => {
  const origin = options?.origin ?? defaultLabCdpOrigin();
  try {
    await waitForCdp(origin, 800);
    return { origin, spawned: null };
  } catch {
    // not up yet
  }
  const command = options?.command ?? defaultLabBrowserCommand();
  const child = NodeChildProcess.spawn(command, [], {
    stdio: "ignore",
    env: process.env,
    detached: false,
  });
  child.unref?.();
  try {
    await waitForCdp(origin, 20_000);
  } catch (cause) {
    child.kill("SIGTERM");
    throw cause;
  }
  return { origin, spawned: child };
};

const currentTab = (
  runtime: LabBrowserRuntime,
  threadId: string,
  requested?: PreviewTabId,
): LabBrowserTab => {
  if (requested) {
    const page = runtime.tabs.get(requested);
    if (!page) throw tabNotFound("status", requested);
    return { tabId: requested, page };
  }
  const assigned = runtime.threadTabs.get(threadId);
  if (assigned) {
    const page = runtime.tabs.get(assigned);
    if (page) return { tabId: assigned, page };
  }
  const first = runtime.tabs.entries().next().value;
  if (!first) throw tabNotFound("status");
  return { tabId: first[0] as PreviewTabId, page: first[1] };
};

const pageStatus = async (
  tab: LabBrowserTab,
  viewportSetting: PreviewViewportSetting,
): Promise<PreviewAutomationStatus> => {
  const page = tab.page;
  const viewport = page.viewportSize() ?? DEFAULT_VIEWPORT;
  return {
    available: true,
    visible: false,
    tabId: tab.tabId,
    url: page.url() || null,
    title: await page.title().catch(() => null),
    loading: false,
    viewportSetting,
    viewport,
  };
};

const locatorOf = (
  page: Page,
  input: { locator?: string | undefined; selector?: string | undefined },
) => {
  if (input.locator) return page.locator(input.locator);
  if (input.selector) return page.locator(input.selector);
  return null;
};

const collectSnapshot = async (page: Page): Promise<PreviewAutomationSnapshot> => {
  const [url, title, visibleText, interactiveElements, accessibilityTree, screenshot] =
    await Promise.all([
      Promise.resolve(page.url()),
      page.title().catch(() => ""),
      page
        .locator("body")
        .innerText()
        .then((text) => text.slice(0, MAX_VISIBLE_TEXT_LENGTH))
        .catch(() => ""),
      page
        .evaluate(
          ({ limit }: { limit: number }) => {
            const nodes = Array.from(
              document.querySelectorAll(
                "a, button, input, textarea, select, [role='button'], [role='link'], [role='textbox']",
              ),
            ).slice(0, limit);
            return nodes.map((node, index) => {
              const el = node as HTMLElement;
              const rect = el.getBoundingClientRect();
              const role = el.getAttribute("role") ?? el.tagName.toLowerCase();
              const name =
                el.getAttribute("aria-label") ||
                el.getAttribute("name") ||
                (el as HTMLInputElement).placeholder ||
                el.textContent?.trim().slice(0, 80) ||
                "";
              return {
                tag: el.tagName.toLowerCase(),
                role,
                name,
                selector: `${el.tagName.toLowerCase()}[data-lab-index="${index}"]`,
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
              };
            });
          },
          { limit: MAX_INTERACTIVE_ELEMENTS },
        )
        .catch(() => []),
      page
        .locator("body")
        .ariaSnapshot()
        .catch(() => null),
      page.screenshot({ type: "png", timeout: 10_000 }),
    ]);
  const viewport = page.viewportSize() ?? DEFAULT_VIEWPORT;
  const width = Math.min(viewport.width, MAX_SCREENSHOT_WIDTH);
  return {
    url,
    title,
    loading: false,
    visibleText,
    interactiveElements,
    accessibilityTree,
    consoleEntries: [],
    networkEntries: [],
    actionTimeline: [],
    screenshot: {
      mimeType: "image/png",
      data: screenshot.toString("base64"),
      width,
      height: viewport.height,
    },
  };
};

export const executeLabPreviewOperation = async (
  runtime: LabBrowserRuntime,
  request: PreviewAutomationRequest,
): Promise<unknown> => {
  const timeoutMs = request.timeoutMs ?? 15_000;
  switch (request.operation) {
    case "status": {
      const tab = currentTab(runtime, request.threadId, request.tabId);
      runtime.threadTabs.set(request.threadId, tab.tabId);
      return await pageStatus(tab, runtime.viewportSetting);
    }
    case "open": {
      const input = request.input as PreviewAutomationOpenInput;
      const reuse = input.reuseExistingTab ?? true;
      let tab: LabBrowserTab;
      if (reuse) {
        try {
          tab = currentTab(runtime, request.threadId, request.tabId);
        } catch {
          tab = bindPage(runtime, await runtime.context.newPage());
        }
      } else {
        tab = bindPage(runtime, await runtime.context.newPage());
      }
      runtime.threadTabs.set(request.threadId, tab.tabId);
      if (input.url) {
        const url = resolveLabNavigationUrl({ url: input.url });
        await tab.page.goto(url, { waitUntil: "load", timeout: timeoutMs });
      }
      return await pageStatus(tab, runtime.viewportSetting);
    }
    case "navigate": {
      const tab = currentTab(runtime, request.threadId, request.tabId);
      const input = request.input as PreviewAutomationNavigateInput;
      const url = resolveLabNavigationUrl(input);
      const readiness = input.readiness ?? "load";
      const waitUntil =
        readiness === "none"
          ? "commit"
          : readiness === "domContentLoaded"
            ? "domcontentloaded"
            : "load";
      await tab.page.goto(url, { waitUntil, timeout: input.timeoutMs ?? timeoutMs });
      runtime.threadTabs.set(request.threadId, tab.tabId);
      return await pageStatus(tab, runtime.viewportSetting);
    }
    case "resize": {
      const tab = currentTab(runtime, request.threadId, request.tabId);
      const input = request.input as PreviewAutomationResizeInput;
      const setting = resolvePreviewViewport(input);
      runtime.viewportSetting = setting;
      const size =
        setting._tag === "fill"
          ? DEFAULT_VIEWPORT
          : { width: setting.width, height: setting.height };
      await tab.page.setViewportSize(size);
      const viewport = tab.page.viewportSize() ?? size;
      return { tabId: tab.tabId, setting, viewport };
    }
    case "setColorScheme": {
      const tab = currentTab(runtime, request.threadId, request.tabId);
      const input = request.input as PreviewAutomationSetColorSchemeInput;
      const colorScheme =
        input.colorScheme === "system" ? null : (input.colorScheme as "light" | "dark");
      await tab.page.emulateMedia({ colorScheme });
      return { tabId: tab.tabId, colorScheme: input.colorScheme };
    }
    case "snapshot": {
      const tab = currentTab(runtime, request.threadId, request.tabId);
      return await collectSnapshot(tab.page);
    }
    case "click": {
      const tab = currentTab(runtime, request.threadId, request.tabId);
      const input = request.input as PreviewAutomationClickInput;
      const loc = locatorOf(tab.page, input);
      if (loc) {
        await loc.click({ timeout: input.timeoutMs ?? timeoutMs });
      } else if (input.x !== undefined && input.y !== undefined) {
        await tab.page.mouse.click(input.x, input.y);
      } else {
        throw new LabHostOperationError(
          "PreviewAutomationInvalidSelectorError",
          `Preview automation click received an invalid selector.`,
        );
      }
      return {};
    }
    case "type": {
      const tab = currentTab(runtime, request.threadId, request.tabId);
      const input = request.input as PreviewAutomationTypeInput;
      const loc = locatorOf(tab.page, input);
      if (loc) {
        if (input.clear) await loc.fill("");
        await loc.fill(input.text, { timeout: input.timeoutMs ?? timeoutMs });
      } else {
        if (input.clear) {
          await tab.page.keyboard.press("Control+A");
          await tab.page.keyboard.press("Backspace");
        }
        await tab.page.keyboard.type(input.text);
      }
      return {};
    }
    case "press": {
      const tab = currentTab(runtime, request.threadId, request.tabId);
      const input = request.input as PreviewAutomationPressInput;
      const chord = [...(input.modifiers ?? []), input.key].join("+");
      await tab.page.keyboard.press(chord);
      return {};
    }
    case "scroll": {
      const tab = currentTab(runtime, request.threadId, request.tabId);
      const input = request.input as PreviewAutomationScrollInput;
      const loc = locatorOf(tab.page, input);
      const deltaX = input.deltaX ?? 0;
      const deltaY = input.deltaY ?? 0;
      if (loc) {
        await loc.evaluate(
          (el, delta) => {
            el.scrollBy(delta.deltaX, delta.deltaY);
          },
          { deltaX, deltaY },
        );
      } else {
        await tab.page.mouse.wheel(deltaX, deltaY);
      }
      return {};
    }
    case "evaluate": {
      const tab = currentTab(runtime, request.threadId, request.tabId);
      const input = request.input as PreviewAutomationEvaluateInput;
      const awaitPromise = input.awaitPromise ?? true;
      const result = await tab.page.evaluate(
        ({ expression, awaitPromise: wait }: { expression: string; awaitPromise: boolean }) => {
          const value = (0, eval)(expression);
          return wait &&
            value !== null &&
            typeof value === "object" &&
            typeof (value as Promise<unknown>).then === "function"
            ? (value as Promise<unknown>)
            : value;
        },
        { expression: input.expression, awaitPromise },
      );
      return result ?? null;
    }
    case "waitFor": {
      const tab = currentTab(runtime, request.threadId, request.tabId);
      const input = request.input as PreviewAutomationWaitForInput;
      const waitTimeout = input.timeoutMs ?? timeoutMs;
      const loc = locatorOf(tab.page, input);
      if (loc) await loc.waitFor({ state: "visible", timeout: waitTimeout });
      if (input.text) {
        await tab.page
          .getByText(input.text)
          .first()
          .waitFor({ state: "visible", timeout: waitTimeout });
      }
      if (input.urlIncludes) {
        await tab.page.waitForURL((url) => url.toString().includes(input.urlIncludes!), {
          timeout: waitTimeout,
        });
      }
      return {};
    }
    case "recordingStart":
    case "recordingStop":
      throw recordingUnavailable(request.operation);
    default:
      throw new LabHostOperationError(
        "PreviewAutomationUnsupportedClientError",
        `Preview automation client ${LAB_CHROMIUM_CLIENT_ID} does not support ${request.operation}.`,
      );
  }
};

export const makeLabPreviewAutomationHost = (
  environmentId: PreviewAutomationHost["environmentId"],
): PreviewAutomationHost => ({
  clientId: LAB_CHROMIUM_CLIENT_ID,
  environmentId,
  supportedOperations: [...PREVIEW_AUTOMATION_OPERATIONS],
});

export const runLabPreviewAutomationHost = (options?: {
  readonly origin?: string;
  readonly command?: string;
  readonly execute?: typeof executeLabPreviewOperation;
  readonly connectRuntime?: typeof attachLabBrowserRuntime;
  readonly ensureChromium?: typeof ensureLabChromium;
}) =>
  Effect.gen(function* () {
    const broker = yield* PreviewAutomationBroker.PreviewAutomationBroker;
    const environment = yield* ServerEnvironment.ServerEnvironment;
    const environmentId = yield* environment.getEnvironmentId;
    const execute = options?.execute ?? executeLabPreviewOperation;
    const ensure = options?.ensureChromium ?? ensureLabChromium;
    const connectRuntime = options?.connectRuntime ?? attachLabBrowserRuntime;

    const handle = yield* Effect.tryPromise({
      try: async () => {
        const ensureOptions: { origin?: string; command?: string } = {};
        if (options?.origin !== undefined) ensureOptions.origin = options.origin;
        if (options?.command !== undefined) ensureOptions.command = options.command;
        const chromiumHandle = await ensure(
          Object.keys(ensureOptions).length > 0 ? ensureOptions : undefined,
        );
        return await connectRuntime(chromiumHandle);
      },
      catch: (cause) => cause,
    }).pipe(
      Effect.tapError((cause) =>
        Effect.logError(
          "Lab Chromium preview host failed to attach; preview_* stays down until retry",
          {
            cause,
          },
        ),
      ),
    );

    const events = yield* broker.connect(makeLabPreviewAutomationHost(environmentId));
    yield* Effect.logInfo("Lab Chromium preview host registered", {
      clientId: LAB_CHROMIUM_CLIENT_ID,
      environmentId,
      cdp: options?.origin ?? defaultLabCdpOrigin(),
    });

    yield* Stream.runForEach(events, (event) =>
      Effect.gen(function* () {
        if (event.type === "connected") {
          yield* broker.focusHost({
            clientId: LAB_CHROMIUM_CLIENT_ID,
            environmentId,
            connectionId: event.connectionId,
            focused: true,
          });
          return;
        }
        const outcome = yield* Effect.result(
          Effect.tryPromise({
            try: () => execute(handle, event.request),
            catch: (cause) => cause,
          }),
        );
        if (Result.isSuccess(outcome)) {
          yield* broker.respond({
            clientId: LAB_CHROMIUM_CLIENT_ID,
            connectionId: event.connectionId,
            requestId: event.request.requestId,
            ok: true,
            ...(outcome.success === undefined ? {} : { result: outcome.success }),
          });
          return;
        }
        yield* broker.respond({
          clientId: LAB_CHROMIUM_CLIENT_ID,
          connectionId: event.connectionId,
          requestId: event.request.requestId,
          ok: false,
          error: serializeLabHostError(outcome.failure, event.request),
        });
      }),
    );
  }).pipe(Effect.withSpan("LabChromiumAutomationHost.run"));

export const layer = Layer.effectDiscard(
  runLabPreviewAutomationHost().pipe(
    Effect.catchCause((cause) => Effect.logError("Lab Chromium preview host stopped", { cause })),
    Effect.forkScoped,
    Effect.asVoid,
  ),
);
