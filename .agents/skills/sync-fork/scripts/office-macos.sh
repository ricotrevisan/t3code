#!/bin/bash
# Runs ON office. Build the arm64 macOS DMG from fork origin/main and serve it
# on the tailnet at https://office.tailedc0c1.ts.net:8443/.
# Prints sha=, dmg=, url=. Never mutates ~/dev/t3code (the live server checkout).
# macOS GUI Tailscale cannot serve files, so a loopback python http.server is
# proxied through tailscale serve on a dedicated port.
set -euo pipefail

TS="/Applications/Tailscale.app/Contents/MacOS/Tailscale"
MISE="/opt/homebrew/bin/mise"
REPO="$HOME/dev/t3code"
WT="$HOME/.t3code/worktrees/sync"
RUN="$HOME/.t3code/run"
HTTP_PORT=8899
SERVE_PORT=8443

mkdir -p "$RUN"
cd "$REPO"
git fetch origin main --quiet

if [[ ! -e "$WT/.git" ]]; then
  if git show-ref --verify --quiet refs/heads/t3-sync; then
    git worktree add "$WT" t3-sync
  else
    git worktree add -b t3-sync "$WT" origin/main
  fi
fi
for state in rebase-merge rebase-apply; do
  path="$(git -C "$WT" rev-parse --git-path "$state")"
  [[ -d "$path" ]] && rm -rf "$path"
done
git -C "$WT" checkout --quiet t3-sync
git -C "$WT" reset --hard origin/main
git -C "$WT" clean -fd -e node_modules -e .t3 -e .env
[[ -f "$REPO/.env" ]] && ln -sf "$REPO/.env" "$WT/.env"
[[ -f "$REPO/infra/relay/.env" ]] && { mkdir -p "$WT/infra/relay"; ln -sf "$REPO/infra/relay/.env" "$WT/infra/relay/.env"; }

SHA="$(git -C "$WT" rev-parse --short HEAD)"

VP="$REPO/node_modules/.bin/vp"
[[ -x "$VP" ]] || { echo "error: vp not found at $VP" >&2; exit 3; }
(cd "$WT" && "$MISE" x node@24.19.0 -- "$VP" i)
(cd "$WT" && "$MISE" x node@24.19.0 -- node scripts/build-desktop-artifact.ts --platform mac --target dmg --arch arm64)

DMG="$(ls -t "$WT"/release/*.dmg 2>/dev/null | head -n 1 || true)"
if [[ -z "$DMG" ]]; then
  echo "error: no DMG produced for ${SHA}" >&2
  exit 4
fi

# Loopback file server for the release dir; reuse it if a previous run left it up.
if ! curl -fsS --max-time 2 "http://127.0.0.1:${HTTP_PORT}/" >/dev/null 2>&1; then
  /usr/bin/python3 -m http.server "$HTTP_PORT" --bind 127.0.0.1 --directory "$WT/release" >"$RUN/dmg-http.log" 2>&1 &
  echo $! > "$RUN/dmg-http.pid"
  sleep 1
fi

"$TS" serve --bg --https="$SERVE_PORT" --yes "http://127.0.0.1:${HTTP_PORT}" >/dev/null

DMG_NAME="${DMG##*/}"
echo "sha=${SHA}"
echo "dmg=${DMG}"
echo "url=https://office.tailedc0c1.ts.net:${SERVE_PORT}/${DMG_NAME}"
