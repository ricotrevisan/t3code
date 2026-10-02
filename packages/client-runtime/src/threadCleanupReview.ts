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
};

/** Shared review lifecycle; each client supplies RPC commands and its own rendering. */
export function createThreadCleanupReview(
  target: ScopedThreadRef,
  commands: {
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
  let state: CleanupState = { review: null, result: null, selected: [], busy: false, error: null };
  let generation = 0;
  const listeners = new Set<() => void>();
  const update = (patch: Partial<CleanupState>) => {
    state = { ...state, ...patch };
    for (const listener of listeners) listener();
  };
  return {
    getSnapshot: () => state,
    subscribe: (listener: () => void) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    dispose: () => {
      generation++;
    },
    reviewAgain: async () => {
      if (state.busy) return;
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
            selected: response.value.actions.filter((a) => !a.blockedReason).map((a) => a.id),
          });
        else update({ error: String(squashAtomCommandFailure(response)) });
      } catch (error) {
        if (current === generation) update({ error: String(error) });
      }
    },
    toggle: (id: ThreadCleanupRunInput["selected"][number], checked: boolean) => {
      if (
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
      const review = state.review;
      if (!review || state.busy || state.result || !state.selected.length) return;
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
