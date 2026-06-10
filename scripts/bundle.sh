#!/usr/bin/env bash
# Build Zorro and assemble dist/Zorro.app (with the icon + Info.plist).
# Usage: scripts/bundle.sh [release|debug]   (default: release)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

PROFILE="${1:-release}"
case "$PROFILE" in
  release) cargo build --release -p zorro; BIN="$ROOT/target/release/zorro" ;;
  debug)   cargo build -p zorro;           BIN="$ROOT/target/debug/zorro" ;;
  *) echo "usage: scripts/bundle.sh [release|debug]"; exit 1 ;;
esac

VERSION="$(grep -m1 '^version' "$ROOT/Cargo.toml" | sed -E 's/.*"(.*)".*/\1/')"
APP="$ROOT/dist/Zorro.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp "$BIN" "$APP/Contents/MacOS/zorro"
chmod +x "$APP/Contents/MacOS/zorro"
cp "$ROOT/crates/zorro/assets/Zorro.icns" "$APP/Contents/Resources/Zorro.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Zorro</string>
  <key>CFBundleDisplayName</key><string>Zorro</string>
  <key>CFBundleIdentifier</key><string>dev.zorro.app</string>
  <key>CFBundleExecutable</key><string>zorro</string>
  <key>CFBundleIconFile</key><string>Zorro</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

plutil -lint "$APP/Contents/Info.plist" >/dev/null
echo "Built $APP (v${VERSION})"
