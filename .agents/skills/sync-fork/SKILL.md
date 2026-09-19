---
name: sync-fork
description: Sync this fork's main with pingdotgg/t3code:main, rebuild iOS and macOS clients on office, and deploy the fork servers across the park. Use when the user asks to sync the fork, run the nightly, pull upstream, rebase onto pingdotgg, refresh builds, or roll out the fork.
---

# Sync fork

Fork `main` stays a rebase of `pingdotgg/t3code:main`: upstream commits are its ancestors and the fork's own commits sit linearly above them. Never merge upstream into fork `main`; a merge also puts upstream in the history but is not a rebase, and it hides fork commits inside a merge. `pingdotgg` is fetch-only; every push goes to `origin`. Never push to or open PRs against `pingdotgg`.

Run every change, Git operation, test, dependency install, Expo authentication, iOS submission, and macOS build on **office (Mac mini)**, which has Xcode and the required macOS/iOS tooling even though mobile uses Expo. Check `uname -s` and `hostname -s`; they must print `Darwin` and `office`. Orchestration may run elsewhere via `ssh office`; execution and artifacts stay on office. The helper scripts reject other hosts.

Use `~/.t3code/worktrees/sync` for sync checkout changes. Preserve the branch and application files in `~/dev/t3code`; it is not necessarily the live runtime (launchd may use a pinned release). Scripts live beside this file; run them with bash.

In the office shell, set:

```bash
REPO="$HOME/dev/t3code"
WT="$HOME/.t3code/worktrees/sync"
SCRIPTS="$HOME/dev/t3code/.agents/skills/sync-fork/scripts"
export PATH="/opt/homebrew/bin:$WT/node_modules/.bin:$PATH"
```

## 1. Check upstream

```
bash "$SCRIPTS/check-upstream.sh"
```

Exit 0: fork `main` is a rebase of upstream. Report the fork and upstream SHAs and stop, unless retrying a skipped build. A build retry uses the already-synced revision directly. Exit 10: new upstream commits were printed; continue. Exit 20: fork `main` contains upstream but carries merge commits; the next sync rebases them away, so report the divergence and continue.

## 2. Prepare the sync worktree

```
bash "$SCRIPTS/sync-worktree.sh"
FROM=$(git -C "$WT" rev-parse origin/main)
```

Prints `worktree=` and `head=`. Keep `FROM` for the push lease and build gates. Completion: the linked worktree belongs to the source repo, is on `t3-sync` at fork `main`, and has dependencies installed. Preparation accepts only a clean worktree whose branch equals or is an ancestor of `origin/main`, and advances it only by fast-forward. Existing environment files/links are retained; only missing links are created.

If preparation refuses, stop and report its reason. On office, inspect `git -C "$WT" status`, `git -C "$WT" log --oneline --left-right origin/main...HEAD`, and `git -C "$REPO" worktree list`. Preserve tracked/untracked files and Git operation metadata in a secure backup outside the checkout before recovery. Have the user choose how to save dirty work, resume an interrupted operation, or retain/reconcile ahead or diverged commits (for example, a named backup branch plus a file backup). A wrong repository/branch requires explicit target correction. Never automatically abort an operation, remove its metadata, reset, clean, or discard work to make preparation pass.

## 3. Rebase

```
git -C "$WT" rebase pingdotgg/main
```

Resolve conflicts keeping upstream intent and re-applying the fork feature on top. When a fork commit fixed a bug upstream has now fixed, verify the upstream diff actually covers the fork fix (read the diff, not the commit message), then drop the commit with `git rebase --skip` and note the upstream commit that replaced it.

Verify the rebase shape before continuing, because a rebase that silently replayed upstream commits, or a merge left in place, both still exit 0:

```
git -C "$WT" merge-base --is-ancestor pingdotgg/main HEAD
test -z "$(git -C "$WT" rev-list --merges pingdotgg/main..HEAD)"
```

Completion: the rebase exits 0, `git -C "$WT" status --porcelain` is empty, upstream is an ancestor, and no merge commits sit above it. Otherwise preserve the in-progress state and report the conflict; resume after resolving it. Aborting or skipping unresolved work requires explicit user approval after a backup. Never force a broken rebase through.

## 4. Prove

Refresh dependencies after rebasing, then run focused tests for the touched files from their package directories so their test setup applies. Use office's managed Node:

```
cd "$WT"
/opt/homebrew/bin/mise x node@24.19.0 -- vp i --frozen-lockfile
/opt/homebrew/bin/mise x node@24.19.0 -- vp test run <touched test files>
```

Completion: every touched test file passes. No repo-wide checks.

## 5. Push

```
git -C "$WT" push --force-with-lease="refs/heads/main:$FROM" origin HEAD:main
```

The rebase rewrote fork commits, so this is intentionally a force push against the fork. Completion: `git -C "$WT" rev-parse origin/main` equals the worktree HEAD.

## 6. Gate the builds

```
cd "$WT" && bash "$SCRIPTS/changed-areas.sh" "$FROM"
```

Prints `mobile=` and `mac=`. `mobile=yes` gates step 7, `mac=yes` gates step 8. Both `no`: skip to the report.

## 7. iOS build (mobile=yes)

Check office's Expo session:

```
bash "$SCRIPTS/office-ios.sh" whoami
bash "$SCRIPTS/office-ios.sh"
```

If `whoami` reports `Not logged in`, report iOS skipped (office has no Expo session) and continue. Other CLI failures are tooling errors, not evidence of missing credentials. The helper runs npm outside the monorepo to avoid its override conflict, then submits from the mobile directory using the fork's `preview` profile. Keep the printed build URL; submission is complete when EAS accepts the build. Do not submit a duplicate when retrying a command that already returned a build URL.

## 8. macOS build (mac=yes)

```
bash "$SCRIPTS/office-macos.sh"
```

The script builds the arm64 DMG from the same clean, pushed revision already prepared and tested in the office sync worktree. It serves the release directory through Tailscale on port 8443 and prints `sha=`, `dmg=`, `url=`. The download URL is stable per app version: `https://office.tailedc0c1.ts.net:8443/<dmg name>`. It replaces only its own port-8443 serve; other Tailscale serves on office stay untouched.

## 9. Deploy the fork servers across the park

Rico has standing authorization for the complete fork rollout. The phrase **“sync the fork”** means rebase, test, push, rebuild every affected client, deploy every fork server, and verify all artifacts and runtimes. Do not stop after GitHub or builds, and do not ask for another deployment confirmation.

The current fork-server fleet is:

- `office`: launchd `com.rico.t3code-prime-rpc`, home `~/.t3code`, Tailscale-bound port `8789`.
- `openclaw`: systemd user unit `t3code-prime-rpc.service`, home `~/.t3code-prime-rpc`, loopback port `8789`, published through existing Tailscale Serve routes.

Never replace or restart `openclaw`'s stock `t3code.service` on port `3773`; it is a separate installation. Before each cutover, discover and verify the live unit, executable, home, listener, and source identity rather than trusting this inventory blindly.

Build the server and web source artifacts from the tested sync worktree on `office`. Target-native dependency installation or native-module compilation may run on the target host when cross-compilation is unsafe; it must use the exact pushed SHA and immutable release directory.

For each server:

1. Capture the old PID, executable/release, service configuration, listener, and HTTP health.
2. Make a consistent SQLite backup and copy the active service configuration.
3. Stage an immutable release keyed by the pushed SHA. Keep the previous release untouched.
4. Verify the staged bundle includes every generated chunk and the web client, and prove required native dependencies load on the target OS.
5. Switch only the named fork service. Never kill by pattern or run a second process against the live home.
6. Verify a new PID executing the staged SHA, service stability across the restart window, HTTP 200 locally and through Tailscale, database quick-check, clean boot logs, and an authenticated project/thread read when available.
7. If any gate fails, restore the prior service configuration/release and repeat the same health checks before continuing.

## 10. Report

Fork `main` SHA, upstream base, dropped commits with their replacements, focused tests, EAS build URL/status, DMG URL, and one verified release/PID/health result per fork server. A sync is incomplete until every applicable build and fleet deployment is either verified or explicitly reported as rolled back/blocked with evidence.
