import { useIsFocused, useFocusEffect } from "@react-navigation/native";
import {
  rollingHourWindow,
  machineResourceValues,
  machineHourlyUsage,
} from "@t3tools/client-runtime/state/machine-overview";
import type { UsageSummaryInput } from "@t3tools/contracts";
import { useCallback, useEffect, useState } from "react";
import { Pressable, View } from "react-native";
import { AppText as Text } from "../../components/AppText";
import { useEnvironments, type EnvironmentPresentation } from "../../state/environments";
import { useThreadShells } from "../../state/entities";
import { useEnvironmentQuery } from "../../state/query";
import { serverEnvironment } from "../../state/server";
import { SettingsScreen } from "../settings/components/SettingsScreen";
import { useHomeThreadSelection } from "../home/home-thread-navigation";

function MachineCard({
  environment,
  window,
  focused,
}: {
  environment: EnvironmentPresentation;
  window: UsageSummaryInput;
  focused: boolean;
}) {
  const connected = environment.connection.phase === "connected";
  const [nowMs, setNowMs] = useState(Date.now);
  const resources = useEnvironmentQuery(
    connected && focused
      ? serverEnvironment.hostResources({ environmentId: environment.environmentId, input: {} })
      : null,
  );
  const usage = useEnvironmentQuery(
    connected && focused
      ? serverEnvironment.usageSummary({ environmentId: environment.environmentId, input: window })
      : null,
  );
  const threads = useThreadShells();
  const selectThread = useHomeThreadSelection();
  const live = threads.filter(
    (thread) => thread.environmentId === environment.environmentId && thread.archivedAt === null,
  );
  const running = live.filter(
    (thread) => thread.session?.status === "running" || thread.session?.status === "starting",
  );
  const refreshResources = resources.refresh;
  useEffect(() => {
    if (!connected || !focused) return;
    const id = setInterval(() => {
      setNowMs(Date.now());
      refreshResources();
    }, 15_000);
    return () => clearInterval(id);
  }, [connected, focused, refreshResources]);
  const { cpu, memory } = machineResourceValues(
    resources.data,
    connected,
    resources.error !== null,
    nowMs,
  );
  const { tokens, partial } = machineHourlyUsage(usage.data);
  return (
    <View className="mb-4 rounded-xl border border-border bg-card p-4">
      <Text className="text-lg font-semibold text-foreground">{environment.label}</Text>
      <Text className="mt-1 text-xs text-muted-foreground">
        {connected
          ? `${running.length} running · ${live.length} threads`
          : environment.connection.phase}
      </Text>
      <View className="my-4 flex-row gap-4">
        {[
          { label: "CPU", value: cpu },
          { label: "RAM", value: memory },
        ].map(({ label, value }) => (
          <View key={label} className="flex-1">
            <Text className="text-sm text-foreground">
              {label}: {value === null ? "Unavailable" : `${Math.round(value)}%`}
            </Text>
            <View className="mt-2 h-2 overflow-hidden rounded bg-muted">
              <View className="h-2 bg-primary" style={{ width: `${value ?? 0}%` }} />
            </View>
          </View>
        ))}
      </View>
      <Text className="text-sm text-foreground">
        Tokens · past hour:{" "}
        {!connected || usage.error || tokens === null
          ? "Unavailable"
          : `${tokens.toLocaleString()}${partial ? " (partial coverage)" : ""}`}
      </Text>
      <Text className="mt-2 text-xs text-muted-foreground">
        Provider usage reported by this environment, including sessions outside T3.
      </Text>
      <Pressable
        accessibilityRole="button"
        disabled={!connected || resources.isPending || usage.isPending}
        onPress={() => {
          resources.refresh();
          usage.refresh();
        }}
        className="my-3 self-end rounded-lg bg-muted px-3 py-2"
      >
        <Text className="text-sm text-foreground">Refresh</Text>
      </Pressable>
      {connected
        ? running.map((thread) => (
            <Pressable
              key={thread.id}
              accessibilityRole="button"
              onPress={() => selectThread(thread)}
              className="border-t border-border py-3"
            >
              <Text numberOfLines={1} className="text-sm text-foreground">
                {thread.title}
              </Text>
            </Pressable>
          ))
        : null}
    </View>
  );
}

export function MachinesRouteScreen() {
  const { environments } = useEnvironments();
  const focused = useIsFocused();
  const [window, setWindow] = useState(() => rollingHourWindow(Date.now()));
  useFocusEffect(
    useCallback(() => {
      setWindow(rollingHourWindow(Date.now()));
      const id = setInterval(() => setWindow(rollingHourWindow(Date.now())), 60_000);
      return () => clearInterval(id);
    }, []),
  );
  return (
    <SettingsScreen title="Machines">
      <Text className="mb-4 text-sm text-muted-foreground">
        Activity and host resources across your environments. Resources refresh every 15 seconds.
      </Text>
      {environments.map((environment) => (
        <MachineCard
          key={environment.environmentId}
          environment={environment}
          window={window}
          focused={focused}
        />
      ))}
      {environments.length === 0 ? (
        <Text className="text-sm text-muted-foreground">
          Connect an environment to see its activity.
        </Text>
      ) : null}
    </SettingsScreen>
  );
}
