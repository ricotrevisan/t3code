#!/usr/bin/env bash
# Summarize what changed between two revisions.
# Usage: changed-areas.sh <from> [to]
# Prints mobile= and mac= gates, then the changed files.
set -euo pipefail

FROM="$1"
TO="${2:-HEAD}"
cd "$(git rev-parse --show-toplevel)"

FILES="$(git diff --name-only "$FROM" "$TO")"
if [[ -z "$FILES" ]]; then
  echo "mobile=no"
  echo "mac=no"
  exit 0
fi

MOBILE="$(grep -c '^apps/mobile/' <<<"$FILES" || true)"
OTHER="$(grep -vc '^apps/mobile/' <<<"$FILES" || true)"

echo "mobile=$([[ "$MOBILE" -gt 0 ]] && echo yes || echo no)"
echo "mac=$([[ "$OTHER" -gt 0 ]] && echo yes || echo no)"
echo "mobile_files=${MOBILE} other_files=${OTHER}"
echo "---"
echo "$FILES"
