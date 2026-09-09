---
name: deploy-t3-lab
description: Deploy or restart the live T3 on lab (systemd t3code.service, ~/.local/lib/t3).
---

# Deploy T3 on lab

This machine (`lab`) serves the T3 people actually use from an **installed package**, not from the git worktree.

- systemd: `t3code.service`
- binary: `/home/rico/.local/lib/t3/dist/bin.mjs`
- home: `/home/rico/.t3` (`T3CODE_HOME`)
- port: `127.0.0.1:3773`

`vp run dev` is a separate worktree server with its own `.t3`. Restarting `t3code.service` without a new pack leaves the old code running.

`AGENTS.md` forbids starting a _new_ server against `~/.t3/userdata`. Updating the **existing** lab service is allowed only when the user asked to put these changes on the live T3.

## Steps

1. **Confirm this is lab's live service.** `hostname` is `lab`. `systemctl is-active t3code.service` is `active`. Capture `old_pid=$(systemctl show -p MainPID --value t3code.service)` and confirm `/proc/$old_pid` cmdline is `.../home/rico/.local/lib/t3/dist/bin.mjs serve --host 127.0.0.1 --port 3773 --base-dir /home/rico/.t3`. Done when those match. If they do not, stop and say so.

2. **Build from the repo root** (`/home/rico/t3code` or the current worktree that holds the change).
   - Server-only change: `vp run --filter t3 build:bundle`. Done when `apps/server/dist/bin.mjs` is newer than the edited source and contains a string unique to the change.
   - Web UI change: also build the web app and copy its `dist` into the live `dist/client` (the packed server looks for `dist/client/index.html`). Done when that `index.html` is newer than the edited UI source.
   - Do not use `vp run dev` for this path.

3. **Backup, then overlay.** `stamp=$(date +%Y%m%d-%H%M)`. Copy `/home/rico/.local/lib/t3/dist` to `/home/rico/.local/lib/t3/dist.backup-$stamp`. Copy the new `apps/server/dist/.` **onto** the live dist. Do **not** `rsync --delete` and do **not** replace the directory in a way that drops `dist/client` unless you rebuilt the client in step 2. Done when live `bin.mjs` matches the new bundle and `dist/client/index.html` still exists.

4. **Restart via systemd.** `sudo systemctl restart t3code.service`. Do not `pkill`, `kill` by name, or start a second `serve` against `~/.t3`. Done when `systemctl is-active` is `active` and MainPID is not `$old_pid`.

5. **Prove the new process.** Wait until `curl -fsS --max-time 2 http://127.0.0.1:3773/` returns HTTP 200 **and** boot log (`~/.t3/userdata/logs/boot-service.log`) shows `Listening on http://127.0.0.1:3773` after the restart timestamp. Quote the new PID and the `provider.session.reaper.started` line (idle threshold should match the packed code). If health fails within ~40s, stop, restore `dist.backup-$stamp` to `dist`, `sudo systemctl start t3code.service`, and report the rollback.

Existing threads reconnect. Prime sessions start fresh on the next user message.
