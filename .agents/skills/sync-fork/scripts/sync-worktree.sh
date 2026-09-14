#!/usr/bin/env bash
# Prepare the office sync worktree at fork main, preserving unfinished work.
# Prints worktree= and head=.
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/office-env.sh"
export GIT_OPTIONAL_LOCKS=0

stop() { echo "error: $*; preserve the worktree and resolve explicitly before retrying" >&2; exit 3; }

check_operation() {
  local state path
  for state in rebase-merge rebase-apply MERGE_HEAD CHERRY_PICK_HEAD REVERT_HEAD sequencer BISECT_START; do
    path="$(git -C "$1" rev-parse --git-path "$state")"
    [[ ! -e "$path" ]] || stop "unfinished Git operation ($state) in $1"
  done
}

check_worktree() {
  local status
  [[ ! -L "$WT" && -f "$WT/.git" ]] || stop "expected a linked worktree at $WT"
  [[ "$(git -C "$WT" rev-parse --show-toplevel)" == "$WT" ]] || stop "wrong worktree root at $WT"
  [[ "$(git -C "$WT" rev-parse --path-format=absolute --git-common-dir)" == "$COMMON" ]] || stop "worktree belongs to another repository"
  check_operation "$WT"
  [[ "$(git -C "$WT" symbolic-ref --quiet HEAD)" == refs/heads/t3-sync ]] || stop "expected branch t3-sync at $WT"
  status="$(git -C "$WT" status --porcelain --untracked-files=all --ignore-submodules=none)"
  [[ -z "$status" ]] || stop "tracked or untracked work exists in $WT"
}

check_base() {
  git rev-parse --verify origin/main^{commit} >/dev/null
  if git show-ref --verify --quiet refs/heads/t3-sync; then
    git merge-base --is-ancestor refs/heads/t3-sync origin/main || stop "t3-sync has ahead or diverged commits"
  fi
}

cd "$REPO"
[[ "$(git rev-parse --show-toplevel)" == "$REPO" ]] || stop "wrong repository root at $REPO"
COMMON="$(git rev-parse --path-format=absolute --git-common-dir)"
check_operation "$REPO"
if [[ -e "$WT" || -L "$WT" ]]; then
  check_worktree
else
  attached="$(git branch --list t3-sync --format='%(worktreepath)')"
  [[ -z "$attached" ]] || stop "t3-sync is checked out elsewhere"
fi
check_base

# Fetch only after local safety checks; remote history may have changed too.
git fetch origin main --quiet
git fetch pingdotgg main --quiet
check_base

if [[ -e "$WT" || -L "$WT" ]]; then
  check_worktree
else
  if git show-ref --verify --quiet refs/heads/t3-sync; then
    git worktree add "$WT" t3-sync
  else
    git worktree add -b t3-sync "$WT" origin/main
  fi
  check_worktree
fi

git -C "$WT" merge --ff-only --no-overwrite-ignore origin/main
[[ "$(git -C "$WT" rev-parse HEAD)" == "$(git rev-parse origin/main)" ]] || stop "worktree did not reach origin/main"

# Keep existing environment files/links, including dangling links. Link only missing ones.
for env in .env infra/relay/.env; do
  if [[ -f "$REPO/$env" && ! -e "$WT/$env" && ! -L "$WT/$env" ]]; then
    mkdir -p "$(dirname "$WT/$env")"
    ln -s "$REPO/$env" "$WT/$env"
  fi
done

VP="$REPO/node_modules/.bin/vp"
[[ -x "$VP" ]] || { echo "error: vp not found at $VP" >&2; exit 3; }
(cd "$WT" && "$MISE" x node@24.19.0 -- "$VP" i --frozen-lockfile)

echo "worktree=${WT}"
echo "head=$(git -C "$WT" rev-parse --short HEAD)"
