#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"
icns="../../../assets/icons/app.icns"
out="assets"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

iconutil --convert iconset "$icns" --output "$tmp/app.iconset"
master="$(ls -S "$tmp/app.iconset"/icon_*.png | head -n1)"

emit() {
  sips -z "$2" "$2" "$master" --out "$out/$1" >/dev/null
}

mkdir -p "$out"

emit StoreLogo.png 50
emit StoreLogo.scale-100.png 50
emit StoreLogo.scale-125.png 63
emit StoreLogo.scale-150.png 75
emit StoreLogo.scale-200.png 100
emit StoreLogo.scale-400.png 200

emit Square150x150Logo.png 150
emit Square150x150Logo.scale-100.png 150
emit Square150x150Logo.scale-125.png 188
emit Square150x150Logo.scale-150.png 225
emit Square150x150Logo.scale-200.png 300
emit Square150x150Logo.scale-400.png 600

emit Square44x44Logo.png 44
emit Square44x44Logo.scale-100.png 44
emit Square44x44Logo.scale-125.png 55
emit Square44x44Logo.scale-150.png 66
emit Square44x44Logo.scale-200.png 88
emit Square44x44Logo.scale-400.png 176

for size in 16 24 32 48 256; do
  emit "Square44x44Logo.targetsize-${size}.png" "$size"
  cp "$out/Square44x44Logo.targetsize-${size}.png" \
    "$out/Square44x44Logo.targetsize-${size}_altform-unplated.png"
done

echo "generated $(ls "$out" | wc -l | tr -d ' ') assets in packaging/windows/appx/assets"
