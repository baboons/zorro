#!/usr/bin/env bash
# Regenerate crates/zorro/assets/Zorro.icns from the 1024x1024 source PNG.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ICON="$ROOT/crates/zorro/assets/icon.png"
SET="$(mktemp -d)/Zorro.iconset"
mkdir -p "$SET"

for s in 16 32 128 256 512; do
  sips -z "$s" "$s"               "$ICON" --out "$SET/icon_${s}x${s}.png"     >/dev/null
  sips -z "$((s * 2))" "$((s * 2))" "$ICON" --out "$SET/icon_${s}x${s}@2x.png" >/dev/null
done

iconutil -c icns "$SET" -o "$ROOT/crates/zorro/assets/Zorro.icns"
echo "Wrote crates/zorro/assets/Zorro.icns"
