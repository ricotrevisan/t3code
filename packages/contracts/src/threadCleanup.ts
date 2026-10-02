import * as Schema from "effect/Schema";
import { IsoDateTime, ThreadId, TrimmedNonEmptyString } from "./baseSchemas.ts";

export const ThreadCleanupActionKind = Schema.Literals(["worktree", "remote-branch", "archive"]);
export const ThreadCleanupAction = Schema.Struct({
  id: ThreadCleanupActionKind,
  title: Schema.String,
  detail: Schema.String,
  blockedReason: Schema.NullOr(Schema.String),
});
export const ThreadCleanupReviewInput = Schema.Struct({ threadId: ThreadId });
export const ThreadCleanupReview = Schema.Struct({
  reviewId: TrimmedNonEmptyString,
  threadId: ThreadId,
  title: Schema.String,
  reviewedAt: IsoDateTime,
  actions: Schema.Array(ThreadCleanupAction),
});
export type ThreadCleanupReview = typeof ThreadCleanupReview.Type;
export const ThreadCleanupRunInput = Schema.Struct({
  threadId: ThreadId,
  reviewId: TrimmedNonEmptyString,
  selected: Schema.Array(ThreadCleanupActionKind).check(Schema.isMaxLength(3)),
});
export type ThreadCleanupRunInput = typeof ThreadCleanupRunInput.Type;
export const ThreadCleanupResult = Schema.Struct({
  threadId: ThreadId,
  actions: Schema.Array(
    Schema.Struct({
      id: ThreadCleanupActionKind,
      status: Schema.Literals(["completed", "deferred", "failed"]),
      detail: Schema.String,
    }),
  ),
});
export type ThreadCleanupResult = typeof ThreadCleanupResult.Type;
export class ThreadCleanupError extends Schema.TaggedError<ThreadCleanupError>()(
  "ThreadCleanupError",
  {
    message: Schema.String,
  },
) {}
