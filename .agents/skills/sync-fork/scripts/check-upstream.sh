#!/usr/bin/env bash
# Compare fork main with pingdotgg/t3code:main.
# Exit 0 = up to date. Exit 10 = new upstream commits (prints them).
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/office-env.sh"
cd "$REPO"

git fetch origin main --quiet
git fetch pingdotgg main --quiet

UP="$(git rev-parse --short pingdotgg/main)"
BEHIND="$(git rev-list --count origin/main..pingdotgg/main)"

if [[ "$BEHIND" -eq 0 ]]; then
  echo "up to date: origin/main ($(git rev-parse --short origin/main)) contains pingdotgg/main (${UP})"
  exit 0
fi

echo "pingdotgg/main (${UP}) is ${BEHIND} commits ahead of origin/main:"
git log --oneline origin/main..pingdotgg/main
exit 10
