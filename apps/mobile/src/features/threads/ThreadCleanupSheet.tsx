import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { Modal, Pressable, ScrollView, View } from "react-native";
import type { ScopedThreadRef } from "@t3tools/contracts";
import { createThreadCleanupReview } from "@t3tools/client-runtime/thread-cleanup-review";
import { AppText } from "../../components/AppText";
import { threadEnvironment } from "../../state/threads";
import { useAtomCommand } from "../../state/use-atom-command";
import { appAtomRegistry } from "../../state/atom-registry";
import { environmentServerConfigsAtom } from "../../state/server";

let present: ((target: ScopedThreadRef) => void) | null = null;
export function supportsThreadCleanup(environmentId: ScopedThreadRef["environmentId"]) {
  return (
    appAtomRegistry.get(environmentServerConfigsAtom).get(environmentId)?.environment.capabilities
      .threadCleanupReview === true
  );
}
export function requestThreadCleanup(target: ScopedThreadRef) {
  if (supportsThreadCleanup(target.environmentId)) present?.(target);
}
export function ThreadCleanupHost() {
  const [target, setTarget] = useState<ScopedThreadRef | null>(null);
  useEffect(() => {
    present = setTarget;
    return () => {
      present = null;
    };
  }, []);
  return target ? (
    <ThreadCleanupSheet
      key={`${target.environmentId}:${target.threadId}`}
      target={target}
      onClose={() => setTarget(null)}
    />
  ) : null;
}
function ThreadCleanupSheet({ target, onClose }: { target: ScopedThreadRef; onClose: () => void }) {
  const inspect = useAtomCommand(threadEnvironment.reviewCleanup, { reportFailure: false });
  const run = useAtomCommand(threadEnvironment.runCleanup, { reportFailure: false });
  const controller = useMemo(
    () =>
      createThreadCleanupReview(
        { environmentId: target.environmentId, threadId: target.threadId },
        { inspect, run },
      ),
    [inspect, run, target.environmentId, target.threadId],
  );
  const { review, result, selected, busy, error } = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
  );
  useEffect(() => {
    void controller.reviewAgain();
    return controller.dispose;
  }, [controller]);
  return (
    <Modal
      visible
      transparent
      animationType="fade"
      onRequestClose={() => {
        if (!busy) onClose();
      }}
    >
      <View className="flex-1 items-center justify-center bg-backdrop px-6">
        <ScrollView
          className="max-h-[85%] w-full max-w-md grow-0 rounded-3xl bg-screen"
          contentContainerStyle={{ padding: 24, gap: 20 }}
        >
          <AppText accessibilityRole="header" className="text-xl font-t3-semibold">
            {result ? "Cleanup results" : "Finish tidying up?"}
          </AppText>
          <AppText>{review?.title ?? "Checking sessions, worktrees, and local files…"}</AppText>
          {error && <AppText accessibilityRole="alert">{error}</AppText>}
          {review?.actions.map((action) => {
            const outcome = result?.actions.find((a) => a.id === action.id);
            return (
              <Pressable
                key={action.id}
                accessibilityRole="checkbox"
                accessibilityState={{
                  checked: selected.includes(action.id),
                  disabled: busy || !!result || !!action.blockedReason,
                }}
                disabled={busy || !!result || !!action.blockedReason}
                className="gap-2 rounded-xl bg-subtle p-4"
                onPress={() => controller.toggle(action.id, !selected.includes(action.id))}
              >
                <AppText className="font-t3-semibold">
                  {!result && (selected.includes(action.id) ? "☑ " : "☐ ")}
                  {action.title}
                </AppText>
                <AppText className="text-foreground-secondary">
                  {result
                    ? (outcome?.detail ?? action.blockedReason ?? "Kept: not selected.")
                    : action.detail}
                </AppText>
                {!result && action.blockedReason && (
                  <AppText>Kept for now: {action.blockedReason}</AppText>
                )}
                {outcome && <AppText>{outcome.status}</AppText>}
              </Pressable>
            );
          })}
          {!result && (
            <AppText className="text-foreground-secondary">
              Each action is checked again before it runs. Nothing happens until you choose Run
              selected.
            </AppText>
          )}
          {!busy && (
            <Pressable
              accessibilityRole="button"
              className="min-h-12 justify-center"
              onPress={() => void controller.reviewAgain()}
            >
              <AppText>Review again</AppText>
            </Pressable>
          )}
          <Pressable
            accessibilityRole="button"
            disabled={busy}
            className="min-h-12 justify-center"
            onPress={onClose}
          >
            <AppText>{result ? "Done" : "Do nothing"}</AppText>
          </Pressable>
          {!result && (
            <Pressable
              accessibilityRole="button"
              disabled={busy || !review || !selected.length}
              className="min-h-12 justify-center rounded-xl bg-subtle px-4"
              onPress={() => void controller.execute()}
            >
              <AppText>{busy ? "Cleaning up…" : `Run ${selected.length} selected`}</AppText>
            </Pressable>
          )}
        </ScrollView>
      </View>
    </Modal>
  );
}
