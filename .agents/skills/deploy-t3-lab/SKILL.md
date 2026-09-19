---
name: deploy-t3-lab
description: Deploy, restart, or roll back the live T3 on lab (systemd t3code.service, immutable release directories selected by ordered systemd drop-ins).
---

# Deploy T3 on lab

`lab` serves the T3 people actually use from an **installed release**, not from a git worktree.

- unit: `t3code.service` (system), fragment `/etc/systemd/system/t3code.service`
- home: `/home/rico/.t3` (`T3CODE_HOME`)
- port: `127.0.0.1:3773`
- releases: `/home/rico/.local/lib/t3/release-<stamp>-<sha>[-patch-<id>]/`

## The one trap

The base unit's `ExecStart` still names the legacy `/home/rico/.local/lib/t3/dist/bin.mjs`. That directory is dead. The live command comes from **drop-ins** in `/etc/systemd/system/t3code.service.d/`, and the **last-sorting file wins**.

A deploy **adds a new drop-in** that sorts after the current last one. It never edits the base unit and never overlays `dist`.

## 1. Read the live identity

```bash
hostname
systemctl is-active t3code.service
OLD_PID=$(systemctl show -p MainPID --value t3code.service)
tr '\0' ' ' < /proc/$OLD_PID/cmdline
systemctl show -p DropInPaths --value t3code.service
```

Done when `hostname` prints `lab`, the unit is `active`, the cmdline names a `release-<stamp>-<sha>` directory, and you can name the drop-in that supplies it. If the cmdline names the legacy `dist/bin.mjs`, stop and report.

## 2. Stage an immutable release

Build the portable server and web client **on office** from the pushed SHA. Lab's checkouts sit at other revisions, so build where the revision is:

```bash
node apps/server/scripts/cli.ts build   # bundles the server, then copies apps/web/dist into dist/client
```

Then, on lab:

- copy that `apps/server/dist` to `release-<stamp>-<sha>/dist`
- copy `package.json` and `package-lock.json` from the live release
- reuse the live release's `node_modules` with `cp -a` when the change adds no dependencies; otherwise install on lab so native modules match the runtime ABI (`/usr/bin/node`)
- write `provenance.json` following the live release's shape: `commit`, `previous_release`, `backup`, `web_identical_file_count`, `native_loads`, `runtime`

Done when the staged `dist/bin.mjs` contains a string unique to your change, `dist/client/index.html` exists, and the client file count equals the live release's.

## 3. Gate the native modules

```bash
/usr/bin/node -e 'const{createRequire}=require("node:module");const r=createRequire(process.argv[1]+"/package.json");for(const m of ["node-pty","msgpackr-extract","@ff-labs/fff-node","playwright-core"]){r(m);console.log("ok",m)}' <new release dir>
sha256sum <new release dir>/node_modules/node-pty/build/Release/pty.node
```

Done when all four modules load **and** `pty.node` hashes the same as the live release's. An `npm ci` warning about blocked install scripts is not proof of a broken native; the hash comparison is.

## 4. Back up, then add the drop-in

```bash
STAMP=$(date +%Y%m%d-%H%M)
BK=/home/rico/.local/state/t3-deploy-backups/$STAMP
mkdir -p "$BK"
systemctl cat t3code.service > "$BK/effective-unit-before.txt"
cp /etc/systemd/system/t3code.service.d/<current last>.conf "$BK/"
```

Write `/etc/systemd/system/t3code.service.d/zzzzz-<intent>-<sha>.conf`:

```
[Service]
ExecStart=
ExecStart=/bin/bash -lc 'GEMINI_API_KEY=$$(/home/rico/.config/varlock/bin/op read "op://Dev/google/add more/gemini") || exit 1; test -n "$$GEMINI_API_KEY" || exit 1; export GEMINI_API_KEY; exec /usr/bin/node <release dir>/dist/bin.mjs serve --host 127.0.0.1 --port 3773 --base-dir /home/rico/.t3'

Environment=T3CODE_SOURCE_SHA=<sha>
Environment=T3CODE_SOURCE_REF=main
Environment=T3CODE_BUILD_KIND=source
```

Copy the `ExecStart` from the drop-in you are replacing and change only the release path. The `$$` escaping is required, and the varlock step must survive verbatim.

Done when the new file sorts last in `systemctl show -p DropInPaths --value t3code.service`.

## 5. Restart and gate

```bash
sudo systemctl daemon-reload
sudo systemctl restart t3code.service
```

Done when **all** of these hold:

- `MainPID` differs from `$OLD_PID`
- `/proc/<new pid>/cmdline` contains the new release path
- `curl -fsS http://127.0.0.1:3773/health` returns 200
- `~/.t3/userdata/logs/boot-service.log` shows `Listening on http://127.0.0.1:3773` after the restart, plus `provider.session.reaper.started`
- `ss -H -ltnp | grep 3773` lists exactly one process

Check the release path explicitly. A restart alone passes a health check on the old release, so health by itself does not prove the cutover.

## 6. Roll back on a failed gate

```bash
sudo rm /etc/systemd/system/t3code.service.d/zzzzz-<intent>-<sha>.conf
sudo systemctl daemon-reload
sudo systemctl restart t3code.service
```

Then re-run every gate from step 5. The previous release directory stays on disk, so rollback is the drop-in removal plus a restart.

## Reference

- A release directory holds `dist/` (server bundle plus `dist/client`), `node_modules/`, `package.json`, `package-lock.json`, `provenance.json`, and `manifest.json` (a relative-path to sha256 map of the release).
- `provenance.json` records `old_release_keep_until`, the retention date for the release it replaced.
- Existing threads reconnect across the restart. Prime sessions start fresh on the next user message.
- `vp run dev` is a separate worktree server with its own `.t3`. Restarting `t3code.service` without a new release leaves the old code running.
- Starting a _new_ server against `~/.t3/userdata` is forbidden. Updating this existing service is the supported path.
