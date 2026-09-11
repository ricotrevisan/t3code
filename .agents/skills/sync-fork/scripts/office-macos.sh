#!/bin/bash
# Runs ON office. Build the arm64 macOS DMG from the prepared sync worktree and serve it
# on the tailnet at https://office.tailedc0c1.ts.net:8443/.
# Prints sha=, dmg=, url=. Never mutates ~/dev/t3code (the live server checkout).
# macOS GUI Tailscale cannot serve files, so a loopback python http.server is
# proxied through tailscale serve on a dedicated port.
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/office-env.sh"
TS="/Applications/Tailscale.app/Contents/MacOS/Tailscale"
RUN="$HOME/.t3code/run"
HTTP_PORT=8899
SERVE_PORT=8443

mkdir -p "$RUN"
[[ -z "$(git -C "$WT" status --porcelain)" ]] || { echo "error: sync worktree is dirty" >&2; exit 3; }
[[ "$(git -C "$WT" rev-parse HEAD)" == "$(git -C "$WT" rev-parse origin/main)" ]] || { echo "error: push the tested sync revision before building" >&2; exit 3; }
SHA="$(git -C "$WT" rev-parse --short HEAD)"
(cd "$WT" && "$MISE" x node@24.19.0 -- /bin/bash -c 'export PATH="/opt/homebrew/bin:'"$WT"'/node_modules/.bin:$PATH"; node scripts/build-desktop-artifact.ts --platform mac --target dmg --arch arm64')

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
