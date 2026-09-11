#!/usr/bin/env bash
# Invoke EAS outside the monorepo so npm does not reject its pnpm overrides.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/office-env.sh"

cd /tmp
if [[ "${1:-}" == whoami ]]; then
  exec "$MISE" x node@24.19.0 -- npx --yes eas-cli whoami
fi

[[ -z "$(git -C "$WT" status --porcelain)" ]] || { echo "error: sync worktree is dirty" >&2; exit 3; }
[[ "$(git -C "$WT" rev-parse HEAD)" == "$(git -C "$WT" rev-parse origin/main)" ]] || { echo "error: push the tested sync revision before building" >&2; exit 3; }
export T3_SYNC_MOBILE_DIR="$WT/apps/mobile"
exec "$MISE" x node@24.19.0 -- npx --yes --package=eas-cli -c 'cd "$T3_SYNC_MOBILE_DIR" && eas build --profile preview -p ios --non-interactive --no-wait'
