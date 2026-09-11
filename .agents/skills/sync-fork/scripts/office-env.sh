#!/usr/bin/env bash
# Shared host and paths for the office sync workflow. Source before doing work.
set -euo pipefail

if [[ "$(uname -s)" != Darwin || "$(hostname -s)" != office ]]; then
  echo "error: run the entire sync-fork workflow on office (Mac mini)" >&2
  exit 2
fi

REPO="$HOME/dev/t3code"
WT="$HOME/.t3code/worktrees/sync"
MISE="/opt/homebrew/bin/mise"
