import { useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import {
  DndContext,
  DragOverlay,
  MeasuringStrategy,
  PointerSensor,
  closestCenter,
  pointerWithin,
  type CollisionDetection,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import { SortableContext, useSortable, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { restrictToVerticalAxis } from "@dnd-kit/modifiers";
import { CSS } from "@dnd-kit/utilities";
import {
  ChevronDownIcon,
  ChevronRightIcon,
  GripVerticalIcon,
  SettingsIcon,
  SquarePenIcon,
} from "lucide-react";
import type { EnvironmentThreadShell } from "@t3tools/client-runtime/state/models";
import type {
  SidebarProjectSnapshot,
  SidebarProjectGroupMember,
} from "../../sidebarProjectGrouping";
import { ProjectFavicon } from "../ProjectFavicon";
import { Button } from "../ui/button";
import { Menu, MenuItem, MenuPopup, MenuTrigger } from "../ui/menu";
import { Tooltip, TooltipPopup, TooltipTrigger } from "../ui/tooltip";
import { cn } from "~/lib/utils";
import { resolveProjectDropIndicator } from "./ProjectThreadTree.logic";

const projectCollisionDetection: CollisionDetection = (args) => {
  const collisions = pointerWithin(args);
  return collisions.length > 0 ? collisions : closestCenter(args);
};

type ProjectGroup = {
  key: string;
  project: SidebarProjectSnapshot;
  threads: readonly EnvironmentThreadShell[];
  running: number;
  waiting: number;
};

type TreeProps = {
  groups: readonly ProjectGroup[];
  collapsed: ReadonlySet<string>;
  onToggle: (key: string) => void;
  onToggleAll: () => void;
  onReorder: (activeKey: string, overKey: string) => void;
  onNewThread: (member: SidebarProjectGroupMember) => void;
  onSettings: (project: SidebarProjectSnapshot) => void;
  renderThreads: (group: ProjectGroup) => ReactNode;
};

/** A sortable project owns its whole branch, so hovering a thread reveals its project's actions. */
function ProjectBranch({
  group,
  expanded,
  dropPosition,
  onKeyboardMove,
  onToggle,
  onNewThread,
  onSettings,
  onPointerDown,
  children,
}: {
  group: ProjectGroup;
  expanded: boolean;
  dropPosition: "before" | "after" | null;
  onKeyboardMove: (direction: -1 | 1) => void;
  onToggle: () => void;
  onNewThread: TreeProps["onNewThread"];
  onSettings: TreeProps["onSettings"];
  onPointerDown: () => void;
  children: ReactNode;
}) {
  const headerRef = useRef<HTMLDivElement>(null);
  const { attributes, listeners, setNodeRef, setActivatorNodeRef, transform, isDragging } =
    useSortable({
      id: group.key,
      data: { headerRef },
      animateLayoutChanges: () => false,
      transition: null,
    });
  const { project } = group;
  const createButton = (
    <Button
      variant="ghost-muted"
      size="icon-xs"
      aria-label={`New thread in ${project.displayName}`}
      onClick={
        project.memberProjects.length === 1
          ? () => onNewThread(project.memberProjects[0]!)
          : undefined
      }
    >
      <SquarePenIcon />
    </Button>
  );
  return (
    <li
      ref={setNodeRef}
      // Project branches have different heights; moving one must never scale its content.
      style={{ transform: CSS.Translate.toString(transform) }}
      className={cn("group/project-branch relative mb-4 list-none", isDragging && "opacity-0")}
    >
      {dropPosition ? (
        <div
          aria-hidden
          className={cn(
            "pointer-events-none absolute inset-x-0 z-40 h-0.5 rounded-full bg-info-foreground",
            dropPosition === "before" ? "-top-2" : "-bottom-2",
          )}
        />
      ) : null}
      <div
        ref={headerRef}
        className={cn(
          "flex items-center gap-0.5 rounded-lg bg-sidebar-row-hover px-1 py-1",
          dropPosition && "ring-1 ring-info-foreground/40",
        )}
      >
        <span className="sr-only touch-none focus-within:not-sr-only pointer-coarse:not-sr-only">
          <Tooltip>
            <TooltipTrigger
              render={
                <Button
                  ref={setActivatorNodeRef}
                  {...attributes}
                  {...listeners}
                  variant="ghost-muted"
                  size="icon-xs"
                  aria-label={`Reorder ${project.displayName}`}
                  aria-describedby={undefined}
                  aria-description="Use ArrowUp and ArrowDown to move this project."
                  aria-keyshortcuts="ArrowUp ArrowDown"
                  onKeyDown={(event) => {
                    if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
                    event.preventDefault();
                    event.stopPropagation();
                    onKeyboardMove(event.key === "ArrowUp" ? -1 : 1);
                  }}
                  onPointerDown={(event) => {
                    onPointerDown();
                    listeners?.onPointerDown?.(event);
                  }}
                >
                  <GripVerticalIcon />
                </Button>
              }
            />
            <TooltipPopup>Drag to reorder, or use the up/down arrow keys</TooltipPopup>
          </Tooltip>
        </span>
        <button
          type="button"
          aria-expanded={expanded}
          aria-label={`${expanded ? "Collapse" : "Expand"} ${project.displayName}`}
          className="flex min-w-0 flex-1 cursor-pointer touch-pan-y items-center gap-2 rounded-md px-1 py-1.5 text-left text-xs font-semibold text-sidebar-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring"
          onPointerDown={(event) => {
            onPointerDown();
            listeners?.onPointerDown?.(event);
          }}
          onClick={onToggle}
          onContextMenu={(event) => {
            event.preventDefault();
            onSettings(project);
          }}
        >
          {expanded ? (
            <ChevronDownIcon aria-hidden className="size-3 shrink-0" />
          ) : (
            <ChevronRightIcon aria-hidden className="size-3 shrink-0" />
          )}
          <ProjectFavicon project={project} className="size-4 shrink-0" />
          <span className="min-w-0 truncate">{project.displayName}</span>
        </button>
        <Tooltip>
          <TooltipTrigger
            render={
              <span
                tabIndex={0}
                className="flex shrink-0 items-center gap-1 text-3xs tabular-nums text-sidebar-muted-foreground"
                aria-label={`${group.threads.length} threads, ${group.running} running, ${group.waiting} waiting for input or approval`}
              />
            }
          >
            <span>{group.threads.length}</span>
            {(group.running > 0 || group.waiting > 0) && (
              <span>
                (<span className="text-info-foreground">{group.running}</span> ·{" "}
                <span className="text-warning-foreground">{group.waiting}</span>)
              </span>
            )}
          </TooltipTrigger>
          <TooltipPopup>
            {group.threads.length} threads · {group.running} running · {group.waiting} waiting for
            input or approval
          </TooltipPopup>
        </Tooltip>
        <div className="flex shrink-0 items-center opacity-0 group-hover/project-branch:opacity-100 group-focus-within/project-branch:opacity-100 pointer-coarse:opacity-100">
          {project.memberProjects.length === 1 ? (
            <Tooltip>
              <TooltipTrigger render={createButton} />
              <TooltipPopup>New thread in {project.displayName}</TooltipPopup>
            </Tooltip>
          ) : (
            <Menu>
              <Tooltip>
                <TooltipTrigger render={<MenuTrigger render={createButton} />} />
                <TooltipPopup>Choose an environment for the new thread</TooltipPopup>
              </Tooltip>
              <MenuPopup side="bottom" align="end">
                {project.memberProjects.map((member) => (
                  <MenuItem key={member.physicalProjectKey} onClick={() => onNewThread(member)}>
                    {member.environmentLabel ?? member.environmentId} — {member.workspaceRoot}
                  </MenuItem>
                ))}
              </MenuPopup>
            </Menu>
          )}
          <Tooltip>
            <TooltipTrigger
              render={
                <Button
                  variant="ghost-muted"
                  size="icon-xs"
                  aria-label={`Settings for ${project.displayName}`}
                  onClick={() => onSettings(project)}
                >
                  <SettingsIcon />
                </Button>
              }
            />
            <TooltipPopup>Project settings</TooltipPopup>
          </Tooltip>
        </div>
      </div>
      {expanded && (
        <ul role="presentation" className="ml-3 border-l-2 border-sidebar-border py-1 pl-3">
          {children}
        </ul>
      )}
    </li>
  );
}

function ProjectDragPreview({ group }: { group: ProjectGroup }) {
  return (
    <div
      aria-hidden
      className="pointer-events-none flex items-center gap-0.5 rounded-lg bg-sidebar px-1 py-1 text-sidebar-foreground shadow-md ring-1 ring-sidebar-border"
    >
      <span className="flex min-w-0 flex-1 items-center gap-2 px-1 py-1.5 text-xs font-semibold">
        <ChevronRightIcon className="size-3 shrink-0" />
        <ProjectFavicon project={group.project} className="size-4 shrink-0" />
        <span className="truncate">{group.project.displayName}</span>
      </span>
      <span className="shrink-0 text-3xs tabular-nums text-sidebar-muted-foreground">
        {group.threads.length}
      </span>
      <span className="w-12 shrink-0" />
    </div>
  );
}

export function ProjectThreadTree(props: TreeProps) {
  const suppressClick = useRef(false);
  const [dragState, setDragState] = useState<{
    activeKey: string;
    overKey: string | null;
    headerRect: DOMRect | null;
  } | null>(null);
  const draggedGroup = dragState
    ? props.groups.find((group) => group.key === dragState.activeKey)
    : null;
  const [keyboardAnnouncement, setKeyboardAnnouncement] = useState("");
  const groupKeys = props.groups.map((group) => group.key);
  const dropIndicator = dragState
    ? resolveProjectDropIndicator(groupKeys, dragState.activeKey, dragState.overKey)
    : null;
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 6 } }));
  const allCollapsed =
    props.groups.length > 0 && props.groups.every((group) => props.collapsed.has(group.key));
  function onDragEnd({ active, over }: DragEndEvent) {
    setDragState(null);
    if (over && active.id !== over.id) props.onReorder(String(active.id), String(over.id));
  }
  return (
    <li className="list-none px-2">
      <span role="status" className="sr-only">
        {keyboardAnnouncement}
      </span>
      <div className="mb-2 flex items-center justify-between">
        <span className="text-xs font-medium text-sidebar-muted-foreground">Projects</span>
        <Button
          variant="ghost-muted"
          size="compact"
          disabled={props.groups.length === 0 || dragState !== null}
          onClick={props.onToggleAll}
        >
          {allCollapsed ? "Expand all" : "Collapse all"}
        </Button>
      </div>
      <DndContext
        sensors={sensors}
        collisionDetection={projectCollisionDetection}
        modifiers={[restrictToVerticalAxis]}
        measuring={{ droppable: { strategy: MeasuringStrategy.Always } }}
        onDragStart={({ active }) => {
          suppressClick.current = true;
          const header = active.data.current?.headerRef?.current;
          // Capture the pointer's viewport grab point before branches collapse.
          const headerRect = header instanceof HTMLElement ? header.getBoundingClientRect() : null;
          setDragState({
            activeKey: String(active.id),
            overKey: null,
            headerRect,
          });
        }}
        onDragOver={({ over }) => {
          const overKey = over ? String(over.id) : null;
          setDragState((current) =>
            current === null || current.overKey === overKey ? current : { ...current, overKey },
          );
        }}
        onDragEnd={onDragEnd}
        onDragCancel={() => {
          setDragState(null);
          suppressClick.current = false;
        }}
      >
        <SortableContext items={groupKeys} strategy={verticalListSortingStrategy}>
          <ul role="presentation">
            {props.groups.map((group) => (
              <ProjectBranch
                key={group.key}
                group={group}
                // Dragging temporarily hides every branch without changing saved collapse state.
                expanded={dragState === null && !props.collapsed.has(group.key)}
                dropPosition={dropIndicator?.key === group.key ? dropIndicator.position : null}
                onKeyboardMove={(direction) => {
                  const index = groupKeys.indexOf(group.key);
                  const target = props.groups[index + direction];
                  if (!target) return;
                  props.onReorder(group.key, target.key);
                  setKeyboardAnnouncement(
                    `Moved ${group.project.displayName} ${direction === -1 ? "before" : "after"} ${target.project.displayName}.`,
                  );
                }}
                onToggle={() => {
                  if (dragState !== null) return;
                  if (suppressClick.current) {
                    suppressClick.current = false;
                    return;
                  }
                  props.onToggle(group.key);
                }}
                onPointerDown={() => {
                  suppressClick.current = false;
                }}
                onNewThread={props.onNewThread}
                onSettings={props.onSettings}
              >
                {dragState === null && !props.collapsed.has(group.key)
                  ? props.renderThreads(group)
                  : null}
              </ProjectBranch>
            ))}
          </ul>
        </SortableContext>
        {createPortal(
          <DragOverlay
            adjustScale={false}
            dropAnimation={null}
            style={
              dragState?.headerRect
                ? {
                    top: dragState.headerRect.top,
                    left: dragState.headerRect.left,
                    width: dragState.headerRect.width,
                    height: "auto",
                  }
                : { height: "auto" }
            }
          >
            {draggedGroup ? <ProjectDragPreview group={draggedGroup} /> : null}
          </DragOverlay>,
          document.body,
        )}
      </DndContext>
    </li>
  );
}
