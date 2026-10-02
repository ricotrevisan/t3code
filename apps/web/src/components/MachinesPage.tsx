import { Link } from "@tanstack/react-router";
import { type UsageSummaryInput } from "@t3tools/contracts";
import { isThreadActivelyRunning } from "@t3tools/client-runtime/state/thread-navigation";
import {
  rollingHourWindow,
  machineResourceValues,
  machineHourlyUsage,
} from "@t3tools/client-runtime/state/machine-overview";
import { useEffect, useMemo, useState } from "react";
import { useNowMinute } from "../hooks/useNowMinute";
import { useEnvironments, type EnvironmentPresentation } from "../state/environments";
import { useThreadShells } from "../state/entities";
import { useEnvironmentQuery } from "../state/query";
import { serverEnvironment } from "../state/server";
import { Button } from "./ui/button";
import { useNavigateToMainApp } from "./sidebar/mainAppLocation";
import { SidebarInset } from "./ui/sidebar";
import { WorkspacePageHeader } from "./WorkspacePageHeader";
import { isElectron } from "../env";

function MachineCard({
  environment,
  window,
}: {
  environment: EnvironmentPresentation;
  window: UsageSummaryInput;
}) {
  const connected = environment.connection.phase === "connected";
  const [nowMs, setNowMs] = useState(Date.now);
  const resources = useEnvironmentQuery(
    connected
      ? serverEnvironment.hostResources({ environmentId: environment.environmentId, input: {} })
      : null,
  );
  const usage = useEnvironmentQuery(
    connected
      ? serverEnvironment.usageSummary({ environmentId: environment.environmentId, input: window })
      : null,
  );
  const threads = useThreadShells();
  const liveThreads = threads.filter(
    (thread) => thread.environmentId === environment.environmentId && thread.archivedAt === null,
  );
  const running = liveThreads.filter(isThreadActivelyRunning);
  const refreshResources = resources.refresh;
  useEffect(() => {
    if (!connected) return;
    const id = setInterval(() => {
      setNowMs(Date.now());
      refreshResources();
    }, 15_000);
    return () => clearInterval(id);
  }, [connected, refreshResources]);
  const { cpu, memory } = machineResourceValues(
    resources.data,
    connected,
    resources.error !== null,
    nowMs,
  );
  const { tokens, partial } = machineHourlyUsage(usage.data);
  return (
    <section className="rounded-xl border border-border bg-card p-5">
      <div className="flex items-center justify-between gap-3">
        <h2 className="truncate text-lg font-semibold">{environment.label}</h2>
        <span className="text-xs text-muted-foreground">
          {connected ? "Connected" : environment.connection.phase}
        </span>
      </div>
      <p className="mt-1 text-sm text-muted-foreground">
        {connected
          ? `${running.length} running · ${liveThreads.length} threads`
          : "Activity unavailable while disconnected"}
      </p>
      <div className="my-5 grid grid-cols-2 gap-4">
        {[
          { label: "CPU", value: cpu },
          { label: "RAM", value: memory },
        ].map(({ label, value }) => (
          <div key={label}>
            <div className="mb-2 flex justify-between text-sm">
              <span>{label}</span>
              <span>{value === null ? "Unavailable" : `${Math.round(value)}%`}</span>
            </div>
            {value === null ? (
              <div aria-hidden className="h-3 rounded bg-muted" />
            ) : (
              <meter
                aria-label={`${label} usage`}
                min={0}
                max={100}
                value={value}
                className="h-3 w-full"
              />
            )}
          </div>
        ))}
      </div>
      <p className="text-sm">
        <span className="font-medium">Tokens · past hour: </span>
        {!connected || usage.error || tokens === null ? "Unavailable" : tokens.toLocaleString()}
        {connected && tokens !== null && partial ? " (partial coverage)" : ""}
      </p>
      <p className="mt-1 text-xs text-muted-foreground">
        Reported provider usage on this host, including sessions outside T3. Unsupported sources are
        not counted.
      </p>
      <div className="mt-4 flex justify-end">
        <Button
          variant="outline"
          size="sm"
          disabled={!connected || resources.isPending || usage.isPending}
          onClick={() => {
            resources.refresh();
            usage.refresh();
          }}
        >
          Refresh
        </Button>
      </div>
      {resources.error || usage.error ? (
        <p role="status" className="mt-2 text-xs text-muted-foreground">
          Some measurements could not be refreshed.
        </p>
      ) : null}
      {connected && running.length > 0 ? (
        <ul className="mt-4 space-y-2 border-t border-border pt-4">
          {running.map((thread) => (
            <li key={thread.id}>
              <Link
                to="/$environmentId/$threadId"
                params={{ environmentId: thread.environmentId, threadId: thread.id }}
                className="block truncate text-sm hover:underline"
              >
                {thread.title}
              </Link>
            </li>
          ))}
        </ul>
      ) : null}
    </section>
  );
}

export function MachinesPage() {
  const { environments } = useEnvironments();
  const minute = useNowMinute();
  const backToThreads = useNavigateToMainApp();
  const window = useMemo(
    () => rollingHourWindow(new Date(`${minute}:00.000Z`).getTime()),
    [minute],
  );
  return (
    <SidebarInset className="h-dvh min-h-0 overflow-hidden">
      <WorkspacePageHeader electron={isElectron}>
        <span className="text-sm font-medium">Machines</span>
      </WorkspacePageHeader>
      <div className="min-h-0 flex-1 overflow-auto p-6">
        <div className="mx-auto max-w-5xl">
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              void backToThreads();
            }}
          >
            ← Back to threads
          </Button>
          <h1 className="mt-4 text-2xl font-semibold">Machines</h1>
          <p className="mt-2 text-sm text-muted-foreground">
            Activity and host resources across your connected environments. Resources refresh every
            15 seconds; usage covers a rolling hour ending at the latest minute.
          </p>
          <div className="mt-6 grid gap-4 md:grid-cols-2">
            {environments.map((environment) => (
              <MachineCard
                key={environment.environmentId}
                environment={environment}
                window={window}
              />
            ))}
          </div>
          {environments.length === 0 ? (
            <p className="mt-6 text-muted-foreground">
              Connect an environment to see its activity.
            </p>
          ) : null}
        </div>
      </div>
    </SidebarInset>
  );
}
