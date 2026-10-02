import type { EnvironmentId, SidebarProjectGroupingMode } from "@t3tools/contracts";
import { DEFAULT_SIDEBAR_PROJECT_SORT_ORDER } from "@t3tools/contracts";
import {
  createContext,
  createElement,
  useCallback,
  useContext,
  useMemo,
  useState,
  type PropsWithChildren,
  type Dispatch,
  type SetStateAction,
} from "react";

import type { HomeProjectSortOrder } from "./homeThreadList";
import type { ThreadNavigationView } from "@t3tools/client-runtime/state/thread-navigation";

export interface HomeListOptions {
  readonly selectedEnvironmentId: EnvironmentId | null;
  readonly projectSortOrder: HomeProjectSortOrder;
  readonly threadView: ThreadNavigationView;
}

export interface ResolvedHomeListOptions extends HomeListOptions {
  readonly projectGroupingMode: SidebarProjectGroupingMode;
}

function defaultHomeListOptions(): HomeListOptions {
  return {
    selectedEnvironmentId: null,
    threadView: "priority",
    projectSortOrder:
      DEFAULT_SIDEBAR_PROJECT_SORT_ORDER === "manual"
        ? "updated_at"
        : DEFAULT_SIDEBAR_PROJECT_SORT_ORDER,
  };
}

interface HomeListOptionsContextValue {
  readonly options: HomeListOptions;
  readonly setOptions: Dispatch<SetStateAction<HomeListOptions>>;
  readonly projectGroupingMode: SidebarProjectGroupingMode;
  readonly collapsedGroups: ReadonlySet<string>;
  readonly setCollapsedGroups: Dispatch<SetStateAction<ReadonlySet<string>>>;
}

const HomeListOptionsContext = createContext<HomeListOptionsContextValue | null>(null);

/** Keeps list preferences stable while the app moves between compact and split shells. */
export function HomeListOptionsProvider({
  children,
  projectGroupingMode,
}: PropsWithChildren<{
  readonly projectGroupingMode: SidebarProjectGroupingMode;
}>) {
  const [options, setOptions] = useState<HomeListOptions>(defaultHomeListOptions);
  const [collapsedGroups, setCollapsedGroups] = useState<ReadonlySet<string>>(new Set());
  const value = useMemo(
    () => ({ options, setOptions, projectGroupingMode, collapsedGroups, setCollapsedGroups }),
    [options, projectGroupingMode, collapsedGroups],
  );
  return createElement(HomeListOptionsContext, { value }, children);
}

export function useThreadNavigationOptions() {
  const shared = useContext(HomeListOptionsContext);
  if (!shared) throw new Error("Thread navigation requires HomeListOptionsProvider");
  const { setOptions, setCollapsedGroups, collapsedGroups, options } = shared;
  const setView = useCallback(
    (threadView: ThreadNavigationView) => setOptions((current) => ({ ...current, threadView })),
    [setOptions],
  );
  const toggleGroup = useCallback(
    (key: string) =>
      setCollapsedGroups((current) => {
        const next = new Set(current);
        if (next.has(key)) next.delete(key);
        else next.add(key);
        return next;
      }),
    [setCollapsedGroups],
  );
  return useMemo(
    () => ({ view: options.threadView, setView, collapsedGroups, toggleGroup }),
    [options.threadView, setView, collapsedGroups, toggleGroup],
  );
}

export function useHomeListOptions(availableEnvironmentIds: ReadonlySet<EnvironmentId>) {
  const shared = useContext(HomeListOptionsContext);
  const [localOptions, setLocalOptions] = useState<HomeListOptions>(defaultHomeListOptions);
  const options = shared?.options ?? localOptions;
  const setOptions = shared?.setOptions ?? setLocalOptions;
  const selectedEnvironmentId =
    options.selectedEnvironmentId !== null &&
    availableEnvironmentIds.has(options.selectedEnvironmentId)
      ? options.selectedEnvironmentId
      : null;
  const availableOptions =
    selectedEnvironmentId === options.selectedEnvironmentId
      ? options
      : { ...options, selectedEnvironmentId };
  const resolvedOptions: ResolvedHomeListOptions = {
    ...availableOptions,
    projectGroupingMode: shared?.projectGroupingMode ?? "repository",
  };

  const setSelectedEnvironmentId = useCallback(
    (value: EnvironmentId | null) => {
      setOptions((current) => ({ ...current, selectedEnvironmentId: value }));
    },
    [setOptions],
  );
  const setProjectSortOrder = useCallback(
    (value: HomeProjectSortOrder) => {
      setOptions((current) => ({ ...current, projectSortOrder: value }));
    },
    [setOptions],
  );
  return {
    options: resolvedOptions,
    setSelectedEnvironmentId,
    setProjectSortOrder,
  } as const;
}
