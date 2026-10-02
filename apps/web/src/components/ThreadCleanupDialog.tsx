import { useEffect, useMemo, useSyncExternalStore } from "react";
import { create } from "zustand";
import type { ScopedThreadRef } from "@t3tools/contracts";
import { createThreadCleanupReview } from "@t3tools/client-runtime/thread-cleanup-review";
import { threadEnvironment } from "../state/threads";
import { useAtomCommand } from "../state/use-atom-command";
import { Button } from "./ui/button";
import { Checkbox } from "./ui/checkbox";
import {
  Dialog,
  DialogPopup,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogPanel,
  DialogFooter,
} from "./ui/dialog";

const useCleanupRequest = create<{ target: ScopedThreadRef | null }>(() => ({ target: null }));
export function requestThreadCleanup(target: ScopedThreadRef) {
  useCleanupRequest.setState({ target });
}

export function ThreadCleanupDialogHost() {
  const target = useCleanupRequest((s) => s.target);
  return target ? (
    <ThreadCleanupDialog key={`${target.environmentId}:${target.threadId}`} target={target} />
  ) : null;
}

function ThreadCleanupDialog({ target }: { target: ScopedThreadRef }) {
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
  const close = () => {
    if (!busy) useCleanupRequest.setState({ target: null });
  };
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) close();
      }}
    >
      <DialogPopup showCloseButton={!busy}>
        <DialogHeader>
          <DialogTitle>{result ? "Cleanup results" : "Finish tidying up?"}</DialogTitle>
          <DialogDescription>{review?.title ?? "Checking this thread…"}</DialogDescription>
        </DialogHeader>
        <DialogPanel>
          {error && (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          )}
          {!review && !error && <p role="status">Checking sessions, worktrees, and local files…</p>}
          {review && (
            <div className="flex flex-col divide-y divide-border">
              {review.actions.map((action) => {
                const outcome = result?.actions.find((a) => a.id === action.id);
                return (
                  <label key={action.id} className="flex items-start gap-3 py-4">
                    {!result && (
                      <Checkbox
                        aria-label={action.title}
                        disabled={busy || !!action.blockedReason}
                        checked={selected.includes(action.id)}
                        onCheckedChange={(checked) =>
                          controller.toggle(action.id, checked === true)
                        }
                      />
                    )}
                    <span className="min-w-0 flex-1">
                      <span className="block text-sm font-medium">{action.title}</span>
                      <span className="mt-1 block break-words text-xs text-muted-foreground">
                        {result
                          ? (outcome?.detail ??
                            action.blockedReason ??
                            "Kept: you did not select this action.")
                          : action.detail}
                      </span>
                      {!result && action.blockedReason && (
                        <span className="mt-2 block text-xs text-muted-foreground">
                          Kept for now: {action.blockedReason}
                        </span>
                      )}
                      {outcome && (
                        <span className="mt-2 block text-xs capitalize">{outcome.status}</span>
                      )}
                    </span>
                  </label>
                );
              })}
            </div>
          )}
          {!result && (
            <p className="mt-3 text-xs text-muted-foreground">
              Each action is checked again before it runs. Nothing happens until you choose Run
              selected.
            </p>
          )}
        </DialogPanel>
        <DialogFooter>
          {!busy && (
            <Button variant="ghost" onClick={() => void controller.reviewAgain()}>
              Review again
            </Button>
          )}
          <Button variant="outline" disabled={busy} onClick={close}>
            {result ? "Done" : "Do nothing"}
          </Button>
          {!result && (
            <Button
              disabled={busy || !review || selected.length === 0}
              onClick={() => void controller.execute()}
            >
              {busy ? "Cleaning up…" : `Run ${selected.length} selected`}
            </Button>
          )}
        </DialogFooter>
      </DialogPopup>
    </Dialog>
  );
}
