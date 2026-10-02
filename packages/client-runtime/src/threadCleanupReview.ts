// @effect-diagnostics globalTimers:off globalDate:off - UI dismissal timing outside an Effect runtime; disposed with the review.
import type {
  ScopedThreadRef,
  ThreadCleanupReview,
  ThreadCleanupResult,
  ThreadCleanupRunInput,
} from "@t3tools/contracts";
import { squashAtomCommandFailure, type AtomCommandResult } from "./state/runtime.ts";

type CleanupState = {
  readonly review: ThreadCleanupReview | null;
  readonly result: ThreadCleanupResult | null;
  readonly selected: readonly ThreadCleanupRunInput["selected"][number][];
  readonly busy: boolean;
  readonly error: string | null;
  readonly remainingSeconds: number | null;
};

/** Shared review lifecycle; each client supplies RPC commands and its own rendering. */
export function createThreadCleanupReview(
  target: ScopedThreadRef,
  commands: {
    dismiss?: () => void;
    inspect: (input: {
      environmentId: ScopedThreadRef["environmentId"];
      input: { threadId: ScopedThreadRef["threadId"] };
    }) => Promise<AtomCommandResult<ThreadCleanupReview, unknown>>;
    run: (input: {
      environmentId: ScopedThreadRef["environmentId"];
      input: ThreadCleanupRunInput;
    }) => Promise<AtomCommandResult<ThreadCleanupResult, unknown>>;
  },
) {
  let state: CleanupState = {
    review: null,
    result: null,
    selected: [],
    busy: false,
    error: null,
    remainingSeconds: null,
  };
  let timer: ReturnType<typeof setInterval> | undefined;
  let interacted = false;
  let dismissed = false;
  let generation = 0;
  const listeners = new Set<() => void>();
  const update = (patch: Partial<CleanupState>) => {
    state = { ...state, ...patch };
    for (const listener of listeners) listener();
  };
  const stopCountdown = () => {
    clearInterval(timer);
    timer = undefined;
    if (state.remainingSeconds !== null) update({ remainingSeconds: null });
  };
  const interact = () => {
    interacted = true;
    stopCountdown();
  };
  const dismiss = () => {
    if (state.busy || dismissed) return;
    dismissed = true;
    generation++;
    stopCountdown();
    commands.dismiss?.();
  };
  const startCountdown = () => {
    stopCountdown();
    if (interacted || !commands.dismiss) return;
    const deadline = Date.now() + 15_000;
    update({ remainingSeconds: 15 });
    timer = setInterval(() => {
      const remainingSeconds = Math.max(0, Math.ceil((deadline - Date.now()) / 1000));
      if (remainingSeconds === 0) dismiss();
      else update({ remainingSeconds });
    }, 1000);
  };
  return {
    interact,
    dismiss,
    getSnapshot: () => state,
    subscribe: (listener: () => void) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    dispose: () => {
      dismissed = true;
      stopCountdown();
      generation++;
    },
    reviewAgain: async () => {
      if (state.busy) return;
      dismissed = false;
      startCountdown();
      const current = ++generation;
      update({ review: null, result: null, selected: [], error: null });
      try {
        const response = await commands.inspect({
          environmentId: target.environmentId,
          input: { threadId: target.threadId },
        });
        if (current !== generation) return;
        if (response._tag === "Success")
          update({
            review: response.value,
            selected: response.value.actions
              .filter((a) => a.id !== "archive" && !a.blockedReason)
              .map((a) => a.id),
          });
        else update({ error: String(squashAtomCommandFailure(response)) });
      } catch (error) {
        if (current === generation) update({ error: String(error) });
      }
    },
    toggle: (id: ThreadCleanupRunInput["selected"][number], checked: boolean) => {
      interact();
      if (
        dismissed ||
        state.busy ||
        state.result ||
        !state.review?.actions.some((a) => a.id === id && !a.blockedReason)
      )
        return;
      update({
        selected: checked
          ? [...new Set([...state.selected, id])]
          : state.selected.filter((selected) => selected !== id),
      });
    },
    execute: async () => {
      interact();
      const review = state.review;
      if (dismissed || !review || state.busy || state.result || !state.selected.length) return;
      const current = generation;
      const selected = review.actions
        .filter((a) => state.selected.includes(a.id) && !a.blockedReason)
        .map((a) => a.id);
      update({ busy: true, error: null });
      try {
        const response = await commands.run({
          environmentId: target.environmentId,
          input: { threadId: target.threadId, reviewId: review.reviewId, selected },
        });
        if (current !== generation) return;
        if (response._tag === "Success") update({ result: response.value });
        else update({ error: String(squashAtomCommandFailure(response)) });
      } catch (error) {
        if (current === generation) update({ error: String(error) });
      } finally {
        if (current === generation) update({ busy: false });
      }
    },
  };
}
