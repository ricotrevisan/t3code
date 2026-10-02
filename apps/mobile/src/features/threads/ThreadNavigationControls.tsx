import { Pressable, View } from "react-native";
import { AppText as Text } from "../../components/AppText";
import { useThreadNavigationOptions } from "../home/home-list-options";

export function ThreadNavigationControls() {
  const { view, setView } = useThreadNavigationOptions();
  return (
    <View className="flex-row gap-2 px-3 py-2">
      {(
        [
          { value: "priority", label: "My priority" },
          { value: "project", label: "By project" },
          { value: "machine", label: "By machine" },
        ] as const
      ).map(({ value, label }) => (
        <Pressable
          key={value}
          accessibilityRole="button"
          accessibilityState={{ selected: view === value }}
          onPress={() => setView(value)}
          className={view === value ? "rounded-lg bg-muted px-3 py-2" : "rounded-lg px-3 py-2"}
        >
          <Text className="text-xs text-foreground">{label}</Text>
        </Pressable>
      ))}
    </View>
  );
}

export function ThreadNavigationGroupHeader({
  groupKey,
  label,
  count,
  running,
  expanded,
}: {
  groupKey: string;
  label: string;
  count: number;
  running: number;
  expanded: boolean;
}) {
  const { toggleGroup } = useThreadNavigationOptions();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityState={{ expanded }}
      onPress={() => toggleGroup(groupKey)}
      className="flex-row items-center gap-2 px-4 py-3"
    >
      <Text className="text-xs text-muted-foreground">{expanded ? "▾" : "▸"}</Text>
      <Text numberOfLines={1} className="flex-1 text-xs font-semibold text-foreground">
        {label}
      </Text>
      <Text className="text-xs text-muted-foreground">
        {running > 0 ? `${running} running · ` : ""}
        {count}
      </Text>
    </Pressable>
  );
}
