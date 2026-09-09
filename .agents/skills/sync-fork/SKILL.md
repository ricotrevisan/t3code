---
name: sync-fork
description: Sync this fork's main with pingdotgg/t3code:main, then rebuild iOS and macOS clients. Use when the user asks to sync the fork, run the nightly, pull upstream, rebase onto pingdotgg, or refresh the iOS/macOS builds.
---

# Sync fork

Fork `main` stays a rebase of `pingdotgg/t3code:main`. `pingdotgg` is fetch-only; every push goes to `origin`. Never push to or open PRs against `pingdotgg`.

All git work happens in disposable worktrees. Live checkouts are off-limits: `/home/rico/t3code` on lab (this session's own checkout) and `~/dev/t3code` on office (the launchd-served office T3). The scripts manage `~/t3code-sync` (lab) and `~/.t3code/worktrees/sync` (office) instead.

Scripts live beside this file; run them with bash. Steps 1-7 run on lab, step 8 on office over ssh.

## 1. Check upstream

```
bash .agents/skills/sync-fork/scripts/check-upstream.sh
```

Exit 0: fork `main` already contains upstream. Report the SHA and stop. Exit 10: new upstream commits were printed; continue.

## 2. Prepare the sync worktree

```
bash .agents/skills/sync-fork/scripts/sync-worktree.sh
```

Prints `worktree=` and `head=`. Record `FROM=$(git -C ~/t3code-sync rev-parse --short origin/main)`; step 6 needs it. Completion: the worktree exists at fork `main` with dependencies installed.

## 3. Rebase

```
git -C ~/t3code-sync rebase pingdotgg/main
```

Resolve conflicts keeping upstream intent and re-applying the fork feature on top. When a fork commit fixed a bug upstream has now fixed, verify the upstream diff actually covers the fork fix (read the diff, not the commit message), then drop the commit with `git rebase --skip` and note the upstream commit that replaced it.

Completion: the rebase exits 0 and `git -C ~/t3code-sync status --porcelain` is empty. Otherwise resolve, or `git rebase --abort` and report the conflict. Never force a broken rebase through.

## 4. Prove

Run focused tests for the files the rebase touched, from the worktree:

```
PATH="$HOME/t3code-sync/node_modules/.bin:$PATH" vp test run <touched test files>
```

Completion: every touched test file passes. No repo-wide checks.

## 5. Push

```
git -C ~/t3code-sync push --force-with-lease origin HEAD:main
```

The rebase rewrote fork commits, so this is intentionally a force push against the fork. Completion: `git -C ~/t3code-sync rev-parse origin/main` equals the worktree HEAD.

## 6. Gate the builds

```
cd ~/t3code-sync && bash .agents/skills/sync-fork/scripts/changed-areas.sh "$FROM"
```

Prints `mobile=` and `mac=`. `mobile=yes` gates step 7, `mac=yes` gates step 8. Both `no`: skip to the report.

## 7. iOS build (mobile=yes)

In `~/t3code-sync/apps/mobile`, check `npx --yes eas-cli whoami`. If it fails, report iOS skipped (no Expo credentials) and continue. Otherwise:

```
npx --yes eas-cli build --profile preview -p ios --non-interactive --no-wait
```

The `preview` profile points at the fork's Expo project. Keep the printed build URL for the report.

## 8. macOS build (mac=yes)

```
scp .agents/skills/sync-fork/scripts/office-macos.sh office:.t3code/bin/office-macos.sh
ssh office 'chmod +x ~/.t3code/bin/office-macos.sh && ~/.t3code/bin/office-macos.sh'
```

The script builds the arm64 DMG on office from the pushed `main` in the office sync worktree, then serves the release directory through Tailscale on port 8443. It prints `sha=`, `dmg=`, `url=`. The download URL is stable per app version: `https://office.tailedc0c1.ts.net:8443/<dmg name>`. It replaces only its own port-8443 serve; other Tailscale serves on office stay untouched.

## 9. Report

Fork `main` SHA, upstream base, dropped commits with their replacements (if any), test result, eas build URL, DMG URL. To move the new server onto live lab, use the deploy-t3-lab skill.
