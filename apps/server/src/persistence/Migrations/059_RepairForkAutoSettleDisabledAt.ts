import migrateAutoSettleDisabledAt from "./054_ProjectionThreadsAutoSettleDisabledAt.ts";

// Historical fork ID 54 ran RepairForkProjectionSchema, not the upstream auto-settle
// migration. Its ledger can advance through 58 while this column remains absent.
// Reapply the idempotent schema change under a new, forward-only migration ID.
export default migrateAutoSettleDisabledAt;
