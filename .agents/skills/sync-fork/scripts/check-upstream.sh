#!/usr/bin/env bash
# Compare fork main with pingdotgg/t3code:main.
# Exit 0 = up to date and rebase-shaped. Exit 10 = new upstream commits (prints them).
# Exit 20 = fork main contains upstream but carries merge commits, so it is not a rebase.
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/office-env.sh"
cd "$REPO"

git fetch origin main --quiet
git fetch pingdotgg main --quiet

UP="$(git rev-parse --short pingdotgg/main)"
BEHIND="$(git rev-list --count origin/main..pingdotgg/main)"

if [[ "$BEHIND" -eq 0 ]]; then
  # fork main is kept as a rebase of upstream, so it must not carry merge commits above it.
  MERGES="$(git rev-list --merges pingdotgg/main..origin/main | wc -l | tr -d ' ')"
  if [[ "$MERGES" -ne 0 ]]; then
    echo "fork main ($(git rev-parse --short origin/main)) contains pingdotgg/main (${UP}) but is not a rebase: ${MERGES} merge commit(s) above it."
    echo "Rebase fork main onto pingdotgg/main; do not merge upstream into it."
    exit 20
  fi
  echo "up to date: origin/main ($(git rev-parse --short origin/main)) is a rebase of pingdotgg/main (${UP})"
  exit 0
fi

echo "pingdotgg/main (${UP}) is ${BEHIND} commits ahead of origin/main:"
git log --oneline origin/main..pingdotgg/main
exit 10
