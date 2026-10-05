// Throwaway: three project-sidebar layouts on the existing page via ?variant=A|B|C.
// All prototype actions are local; no project or thread mutations are sent.
import { useEffect, useState } from "react";
import {
  ChevronDownIcon,
  ChevronRightIcon,
  FolderIcon,
  SettingsIcon,
  SquarePenIcon,
  XIcon,
} from "lucide-react";
import { cn } from "~/lib/utils";

const variants = ["A", "B", "C"] as const;
type Variant = (typeof variants)[number];
const names = { A: "Nested list", B: "Project cards", C: "Workspace tree" };

export function useProjectViewPrototype() {
  const [variant, setVariant] = useState<Variant | null>(() => {
    if (!import.meta.env.DEV) return null;
    const value = new URLSearchParams(window.location.search).get("variant");
    return variants.find((item) => item === value) ?? null;
  });
  function changeVariant(next: Variant) {
    const url = new URL(window.location.href);
    url.searchParams.set("variant", next);
    window.history.replaceState(window.history.state, "", url);
    setVariant(next);
  }
  return { variant, changeVariant };
}

type Project = {
  key: string;
  label: string;
  threads: { key: string; title: string; running: boolean; selected: boolean }[];
};

export function ProjectViewPrototype({
  projects,
  variant,
  changeVariant,
}: {
  projects: Project[];
  variant: Variant;
  changeVariant: (variant: Variant) => void;
}) {
  const [collapsed, setCollapsed] = useState(new Set<string>());
  const [drafts, setDrafts] = useState<Record<string, string[]>>({});
  const [selected, setSelected] = useState<string | null>(null);
  const [settings, setSettings] = useState<Project | null>(null);
  const [action, setAction] = useState("No preview actions yet");
  const cycle = (direction: number) =>
    changeVariant(
      variants[(variants.indexOf(variant) + direction + variants.length) % variants.length]!,
    );
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (
        event.target instanceof HTMLElement &&
        event.target.closest("input, textarea, select, [contenteditable]")
      )
        return;
      if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
        event.preventDefault();
        changeVariant(
          variants[
            (variants.indexOf(variant) + (event.key === "ArrowRight" ? 1 : 2)) % variants.length
          ]!,
        );
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [variant, changeVariant]);

  const iconButton =
    "flex size-7 shrink-0 items-center justify-center rounded-md text-sidebar-muted-foreground hover:bg-sidebar-row-hover hover:text-sidebar-foreground focus-visible:outline-2 focus-visible:outline-ring";
  return (
    <div className="px-2 pb-4 text-sidebar-foreground">
      {projects.map((project) => {
        const open = !collapsed.has(project.key);
        const threads = [
          ...project.threads,
          ...(drafts[project.key] ?? []).map((key) => ({
            key,
            title: "New thread",
            running: false,
            selected: false,
          })),
        ];
        return (
          <section
            key={project.key}
            className={cn(
              "mb-3",
              variant === "B" &&
                "overflow-hidden rounded-xl border border-sidebar-border bg-background/40",
              variant === "C" && "mb-5",
            )}
          >
            <div
              className={cn(
                "flex items-center gap-0.5",
                variant === "A" && "py-1",
                variant === "B" && "border-b border-sidebar-border px-2 py-2",
                variant === "C" && "rounded-lg bg-sidebar-row-hover px-1 py-1",
              )}
            >
              <button
                type="button"
                aria-expanded={open}
                onClick={() => {
                  setCollapsed((current) => {
                    const next = new Set(current);
                    if (next.has(project.key)) next.delete(project.key);
                    else next.add(project.key);
                    return next;
                  });
                  setAction(`${open ? "Collapsed" : "Expanded"} ${project.label}`);
                }}
                className="flex min-w-0 flex-1 items-center gap-2 rounded-md px-1 py-1.5 text-left text-xs font-semibold hover:bg-sidebar-row-hover"
              >
                {open ? (
                  <ChevronDownIcon className="size-3 shrink-0" />
                ) : (
                  <ChevronRightIcon className="size-3 shrink-0" />
                )}
                <FolderIcon className="size-4 shrink-0 text-sidebar-muted-foreground" />
                <span className="truncate">{project.label}</span>
                {variant !== "A" && (
                  <span className="ml-auto text-3xs font-normal text-sidebar-muted-foreground">
                    {threads.length}
                  </span>
                )}
              </button>
              <button
                type="button"
                className={iconButton}
                aria-label={`New thread in ${project.label}`}
                onClick={() => {
                  const key = `${project.key}:draft:${(drafts[project.key] ?? []).length + 1}`;
                  setDrafts((current) => ({
                    ...current,
                    [project.key]: [...(current[project.key] ?? []), key],
                  }));
                  setCollapsed((current) => {
                    const next = new Set(current);
                    next.delete(project.key);
                    return next;
                  });
                  setSelected(key);
                  setAction(`Created preview thread in ${project.label}`);
                }}
              >
                <SquarePenIcon className="size-3.5" />
              </button>
              <button
                type="button"
                className={iconButton}
                aria-label={`Settings for ${project.label}`}
                onClick={() => {
                  setSettings(project);
                  setAction(`Opened settings for ${project.label}`);
                }}
              >
                <SettingsIcon className="size-3.5" />
              </button>
            </div>
            {open && (
              <div
                className={cn(
                  variant === "A" && "ml-[23px] border-l border-sidebar-border pl-2",
                  variant === "B" && "space-y-1 p-2",
                  variant === "C" && "ml-3 border-l-2 border-sidebar-border py-1 pl-3",
                )}
              >
                {threads.map((thread) => (
                  <button
                    key={thread.key}
                    type="button"
                    onClick={() => {
                      setSelected(thread.key);
                      setAction(`Selected ${project.label} / ${thread.title} (preview only)`);
                    }}
                    className={cn(
                      "flex w-full items-center gap-2 rounded-md px-2 py-2 text-left text-xs hover:bg-sidebar-row-hover",
                      (selected ? selected === thread.key : thread.selected) &&
                        "bg-accent text-accent-foreground",
                      variant === "B" && "border border-sidebar-border",
                      variant === "B" &&
                        !(selected ? selected === thread.key : thread.selected) &&
                        "bg-sidebar",
                    )}
                  >
                    {variant === "C" && <span className="text-sidebar-muted-foreground">└</span>}
                    <span className="min-w-0 flex-1">
                      <span className="block truncate">{thread.title}</span>
                      {variant === "B" && (
                        <span className="mt-1 block text-3xs text-sidebar-muted-foreground">
                          {thread.running ? "Running" : "Ready"} · {project.label}
                        </span>
                      )}
                    </span>
                    {thread.running && (
                      <span
                        className="size-1.5 shrink-0 rounded-full bg-success"
                        aria-label="Running"
                      />
                    )}
                  </button>
                ))}
                {threads.length === 0 && (
                  <p className="px-2 py-3 text-xs text-sidebar-muted-foreground">No threads yet</p>
                )}
                {variant === "B" && (
                  <p className="px-2 py-2 text-3xs text-sidebar-muted-foreground">
                    Threads stay inside this project
                  </p>
                )}
              </div>
            )}
          </section>
        );
      })}
      {settings && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/30"
          onClick={() => setSettings(null)}
        >
          <div
            role="dialog"
            aria-modal="true"
            aria-label={`Preview settings for ${settings.label}`}
            className="w-80 rounded-xl border border-border bg-background p-5 shadow-xl"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="mb-4 flex items-center justify-between">
              <h2 className="text-sm font-semibold">{settings.label}</h2>
              <button
                type="button"
                className={iconButton}
                aria-label="Close settings"
                onClick={() => setSettings(null)}
              >
                <XIcon className="size-4" />
              </button>
            </div>
            <p className="text-xs text-muted-foreground">
              Project settings preview. No changes are saved.
            </p>
            <dl className="mt-4 space-y-2 text-xs">
              <div>
                <dt className="text-muted-foreground">Project</dt>
                <dd>{settings.label}</dd>
              </div>
              <div>
                <dt className="text-muted-foreground">Threads</dt>
                <dd>{settings.threads.length + (drafts[settings.key]?.length ?? 0)}</dd>
              </div>
            </dl>
          </div>
        </div>
      )}
      <div className="fixed bottom-5 left-1/2 z-40 flex max-w-[95vw] -translate-x-1/2 flex-col items-center gap-1 rounded-xl bg-foreground px-3 py-2 text-background shadow-lg">
        <div className="flex items-center gap-4">
          <button type="button" aria-label="Previous variant" onClick={() => cycle(-1)}>
            ←
          </button>
          <span className="text-xs font-semibold">
            PROTOTYPE · {variant} · {names[variant]}
          </span>
          <button type="button" aria-label="Next variant" onClick={() => cycle(1)}>
            →
          </button>
        </div>
        <span className="text-3xs opacity-70">Local only · {action}</span>
        <span className="max-w-lg truncate text-3xs opacity-70">
          State: collapsed {collapsed.size} · drafts {Object.values(drafts).flat().length} ·
          selected {selected ?? "current thread"}
        </span>
      </div>
    </div>
  );
}
