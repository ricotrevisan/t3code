---
name: sync-fork
description: Sync this fork's main with pingdotgg/t3code:main and rebuild iOS and macOS clients, entirely on office (Mac mini). Use when the user asks to sync the fork, run the nightly, pull upstream, rebase onto pingdotgg, or refresh the iOS/macOS builds.
---

# Sync fork

Fork `main` stays a rebase of `pingdotgg/t3code:main`. `pingdotgg` is fetch-only; every push goes to `origin`. Never push to or open PRs against `pingdotgg`.

Run every step on **office (Mac mini)**: fetch, rebase, tests, push, Expo authentication, iOS submission, and macOS packaging. Check `hostname -s`; it must print `office`. When the session is already on office, run locally. Otherwise use `ssh office`. The helper scripts reject other hosts.

Use `~/.t3code/worktrees/sync` for all checkout changes. `~/dev/t3code` serves the live office T3; leave its branch and application files alone. Scripts live beside this file; run them with bash.

In the office shell, set:

```bash
WT="$HOME/.t3code/worktrees/sync"
SCRIPTS="$HOME/dev/t3code/.agents/skills/sync-fork/scripts"
export PATH="/opt/homebrew/bin:$WT/node_modules/.bin:$PATH"
```

## 1. Check upstream

```
bash "$SCRIPTS/check-upstream.sh"
```

Exit 0: fork `main` already contains upstream. Report the fork and upstream SHAs and stop, unless retrying a skipped build. A build retry uses the already-synced revision directly. Exit 10: new upstream commits were printed; continue.

## 2. Prepare the sync worktree

```
bash "$SCRIPTS/sync-worktree.sh"
FROM=$(git -C "$WT" rev-parse origin/main)
```

Prints `worktree=` and `head=`. Keep `FROM` for the push lease and build gates. Completion: the worktree exists at fork `main` with dependencies installed.

## 3. Rebase

```
git -C "$WT" rebase pingdotgg/main
```

Resolve conflicts keeping upstream intent and re-applying the fork feature on top. When a fork commit fixed a bug upstream has now fixed, verify the upstream diff actually covers the fork fix (read the diff, not the commit message), then drop the commit with `git rebase --skip` and note the upstream commit that replaced it.

Completion: the rebase exits 0 and `git -C "$WT" status --porcelain` is empty. Otherwise resolve, or `git rebase --abort` and report the conflict. Never force a broken rebase through.

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

## 9. Report

Fork `main` SHA, upstream base, dropped commits with their replacements (if any), test result, EAS build URL, DMG URL. Live server deployment is a separate task.
