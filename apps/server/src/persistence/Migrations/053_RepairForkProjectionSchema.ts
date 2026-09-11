import * as Effect from "effect/Effect";

import migrateThreadPullRequests from "./050_ProjectionThreadPullRequests.ts";
import migrateThreadMessageContext from "./051_ProjectionThreadMessageContext.ts";

// Fork installs used IDs 51 and 52 before upstream added message context at 51.
// Repair above both historical IDs; both migrations are idempotent.
export default Effect.gen(function* () {
  yield* migrateThreadPullRequests;
  yield* migrateThreadMessageContext;
});
