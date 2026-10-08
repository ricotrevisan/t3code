#!/usr/bin/env bash
# Wraps a built native client in a minimal, ad-hoc signed macOS app bundle so
# bundle-only platform features (user notifications, Dock badge) can run.
#
#   apps/desktop-gpui/parity/bundle-macos.sh [debug|release] [output-dir]
#
# Launch the bundled executable directly (Contents/MacOS/T3 Code Native) to pass
# environment variables; macOS still resolves the surrounding bundle.
set -euo pipefail

profile="${1:-debug}"
out_dir="${2:-$(dirname "$0")/../target/$profile}"
crate_dir="$(cd "$(dirname "$0")/.." && pwd)"
binary="$crate_dir/target/$profile/t3code-desktop-gpui"
app="$out_dir/T3 Code Native.app"

[[ -x "$binary" ]] || { echo "missing $binary; build it first" >&2; exit 1; }

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/T3 Code Native"
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>codes.t3.native.dev</string>
  <key>CFBundleName</key><string>T3 Code Native</string>
  <key>CFBundleDisplayName</key><string>T3 Code Native</string>
  <key>CFBundleExecutable</key><string>T3 Code Native</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
codesign --force --sign - "$app" >/dev/null
echo "$app"
