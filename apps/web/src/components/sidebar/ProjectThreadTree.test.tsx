// @vitest-environment jsdom
import { act, useState } from "react";
import { arrayMove } from "@dnd-kit/sortable";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import { EnvironmentId, ProjectId, ProviderInstanceId } from "@t3tools/contracts";
import { scopeProjectRef } from "@t3tools/client-runtime/environment";
import type { SidebarProjectSnapshot } from "../../sidebarProjectGrouping";
import { ProjectThreadTree } from "./ProjectThreadTree";

vi.mock("../ProjectFavicon", () => ({ ProjectFavicon: () => null }));

const environmentId = EnvironmentId.make("local");
function project(name: string): SidebarProjectSnapshot {
  const member = {
    id: ProjectId.make(name),
    environmentId,
    title: name,
    workspaceRoot: `/test/${name}`,
    repositoryIdentity: null,
    defaultModelSelection: { instanceId: ProviderInstanceId.make("codex"), model: "gpt-5.4" },
    scripts: [],
    createdAt: "2026-10-06T10:00:00Z",
    updatedAt: "2026-10-06T10:00:00Z",
    physicalProjectKey: `local:${name}`,
    environmentLabel: "Local",
  };
  return {
    ...member,
    projectKey: name,
    displayName: name,
    groupedProjectCount: 1,
    environmentPresence: "local-only",
    allRemoteMembersAreDesktopLocal: false,
    allRemoteMembersAreWsl: false,
    memberProjects: [member],
    memberProjectRefs: [scopeProjectRef(environmentId, member.id)],
    remoteEnvironmentLabels: [],
  };
}
const groups = ["First", "Middle", "Last"].map((name) => ({
  key: `project:${name}`,
  project: project(name),
  threads: [],
  running: 0,
  waiting: 0,
}));

class TestPointerEvent extends MouseEvent {
  pointerId = 1;
  isPrimary = true;
  pointerType = "mouse";
}
let root: Root;
let container: HTMLDivElement;

function TreeHarness() {
  const [orderedGroups, setOrderedGroups] = useState(groups);
  return (
    <ul>
      <ProjectThreadTree
        groups={orderedGroups}
        collapsed={new Set(["project:Middle"])}
        onToggle={() => {}}
        onToggleAll={() => {}}
        onReorder={(active, over) =>
          setOrderedGroups((current) =>
            arrayMove(
              current,
              current.findIndex((group) => group.key === active),
              current.findIndex((group) => group.key === over),
            ),
          )
        }
        onNewThread={() => {}}
        onSettings={() => {}}
        renderThreads={(group) => <li>Content for {group.project.displayName}</li>}
      />
    </ul>
  );
}

// jsdom has no layout. Model branches as 36px headers + 160px expanded content,
// with real DOM expansion deciding their geometry. Fixed overlays honor their CSS
// position/translation so we can detect cursor drift after preceding branches shrink.
function bounds(element: HTMLElement) {
  if (element.style.position === "fixed") {
    const translation = /translate3d\(([-\d.]+)px,\s*([-\d.]+)px/.exec(element.style.transform);
    return new DOMRect(
      Number.parseFloat(element.style.left) + Number(translation?.[1] ?? 0),
      Number.parseFloat(element.style.top) + Number(translation?.[2] ?? 0),
      Number.parseFloat(element.style.width),
      36,
    );
  }
  if (element.parentElement?.style.position === "fixed") return bounds(element.parentElement);
  const branch = element.closest("li.group\\/project-branch");
  if (!branch) return new DOMRect(20, 100, 240, 600);
  let top = 100;
  for (const candidate of container.querySelectorAll("li.group\\/project-branch")) {
    const height = 36 + (candidate.querySelector("ul") ? 160 : 0);
    if (candidate === branch) return new DOMRect(20, top, 240, element === branch ? height : 36);
    top += height + 16;
  }
  return new DOMRect();
}

beforeEach(async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("PointerEvent", TestPointerEvent);
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
  container = document.createElement("div");
  document.body.append(container);
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement,
  ) {
    return bounds(this);
  });
  root = createRoot(container);
  await act(() => root.render(<TreeHarness />));
});

async function keyboardDrag(direction: "ArrowUp" | "ArrowDown" | null, name = "Middle") {
  const handle = container.querySelector(`button[aria-label="Reorder ${name}"]`);
  if (!(handle instanceof HTMLButtonElement)) throw new Error("Missing reorder handle");
  await act(() => handle.focus());
  if (direction)
    await act(() =>
      handle.dispatchEvent(
        new KeyboardEvent("keydown", { bubbles: true, code: direction, key: direction }),
      ),
    );
  expect(document.activeElement).toBe(handle);
  return [...container.querySelectorAll("li.group\\/project-branch button[aria-expanded]")].map(
    (header) => header.textContent,
  );
}

it.each(["First", "Middle", "Last"])(
  "does not move focused %s until an arrow key is pressed",
  async (name) => {
    expect(await keyboardDrag(null, name)).toEqual(["First", "Middle", "Last"]);
  },
);

it.each([
  ["Middle", "ArrowUp", ["Middle", "First", "Last"]],
  ["Middle", "ArrowDown", ["First", "Last", "Middle"]],
  ["First", "ArrowDown", ["Middle", "First", "Last"]],
  ["Last", "ArrowUp", ["First", "Last", "Middle"]],
] as const)(
  "moves focused %s exactly one position with %s among mixed expanded branches",
  async (name, direction, expected) => {
    expect(await keyboardDrag(direction, name)).toEqual(expected);
  },
);

afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it.each(["pointerup", "pointercancel"])(
  "keeps a bottom project's grab point under the cursor and restores branches after %s",
  async (finish) => {
    const header = container.querySelector('button[aria-label="Collapse Last"]');
    if (!(header instanceof HTMLButtonElement)) throw new Error("Missing project header");
    const original = header.getBoundingClientRect();
    const grabOffset = 12;
    const initialY = original.top + grabOffset;
    await act(() =>
      header.dispatchEvent(
        new TestPointerEvent("pointerdown", {
          bubbles: true,
          button: 0,
          clientX: 80,
          clientY: initialY,
        }),
      ),
    );
    await act(() =>
      document.dispatchEvent(
        new TestPointerEvent("pointermove", { bubbles: true, clientX: 80, clientY: initialY + 8 }),
      ),
    );
    expect(container.textContent).not.toContain("Content for First");
    expect(container.textContent).not.toContain("Content for Last");
    const currentY = initialY + 30;
    await act(() =>
      document.dispatchEvent(
        new TestPointerEvent("pointermove", { bubbles: true, clientX: 80, clientY: currentY }),
      ),
    );
    const preview = [...document.body.querySelectorAll("div")].find(
      (element) =>
        element.style.position === "fixed" &&
        element.querySelector(':scope > div[aria-hidden="true"]')?.textContent?.includes("Last"),
    );
    if (!preview) throw new Error("Missing drag preview");
    expect(preview.getBoundingClientRect().top + grabOffset).toBe(currentY);
    await act(() =>
      document.dispatchEvent(
        new TestPointerEvent(finish, { bubbles: true, clientX: 80, clientY: currentY }),
      ),
    );
    expect(container.textContent).toContain("Content for First");
    expect(container.textContent).toContain("Content for Last");
    expect(container.textContent).not.toContain("Content for Middle");
    expect(preview.isConnected).toBe(false);
  },
);
