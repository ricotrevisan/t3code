#!/usr/bin/env bash
# Prepare the disposable lab sync worktree at fork main.
# Prints worktree= and head=.
set -euo pipefail

REPO="${1:-$HOME/t3code}"
WT="${T3_SYNC_WORKTREE:-$HOME/t3code-sync}"

cd "$REPO"
git fetch origin main --quiet
git fetch pingdotgg main --quiet

if [[ ! -e "$WT/.git" ]]; then
  if git show-ref --verify --quiet refs/heads/t3-sync; then
    git worktree add "$WT" t3-sync
  else
    git worktree add -b t3-sync "$WT" origin/main
  fi
fi

# A previous run may have died mid-rebase; clear that before resetting.
for state in rebase-merge rebase-apply; do
  path="$(git -C "$WT" rev-parse --git-path "$state")"
  if [[ -d "$path" ]]; then
    git -C "$WT" rebase --abort >/dev/null 2>&1 || rm -rf "$path"
  fi
done

git -C "$WT" checkout --quiet t3-sync
git -C "$WT" reset --hard origin/main
git -C "$WT" clean -fd -e node_modules -e .t3 -e .env

# Worktree setup mirrors t3.json: .env comes from the project root.
[[ -f "$REPO/.env" ]] && ln -sf "$REPO/.env" "$WT/.env"

VP="$REPO/node_modules/.bin/vp"
if [[ -x "$VP" && ! -x "$WT/node_modules/.bin/vp" ]]; then
  (cd "$WT" && "$VP" i)
fi

echo "worktree=${WT}"
echo "head=$(git -C "$WT" rev-parse --short HEAD)"
