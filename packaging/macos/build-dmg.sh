#!/bin/bash
# Builds dist/Lantern-x.y.z.dmg on a Mac, for this Mac's architecture
# (one compile). UNIVERSAL=1 builds Apple Silicon + Intel. The DMG is only
# made again when the program changed. The list of what belongs to this
# build: dist/files-mac.txt.
set -euo pipefail
cd "$(dirname "$0")/../.."
# shellcheck source=../../scripts/ui.sh
. scripts/ui.sh
[ "$(uname -s)" = "Darwin" ] || ui_fail "This script must run on macOS."
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"
command -v cargo >/dev/null 2>&1 || export PATH="$HOME/.cargo/bin:$PATH"
V=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
mkdir -p dist
ui_header "Lantern $V" "Mac bauen"

if [ "$(sysctl -n hw.optional.arm64 2>/dev/null || echo 0)" = "1" ]; then HOST=arm64; else HOST=x86_64; fi
ARCHS="${ARCHS:-$HOST}"
[ -n "${UNIVERSAL:-}" ] && ARCHS="arm64 x86_64"
BINS=()
for a in $ARCHS; do
  case "$a" in
    arm64|aarch64) a=arm64; t=aarch64-apple-darwin ;;
    *) a=x86_64; t=x86_64-apple-darwin ;;
  esac
  if [ "$a" = "$HOST" ] && [ "$(uname -m)" = "$HOST" ]; then
    ui_run "Programm für diesen Mac ($a)" cargo build --release --locked
    BINS+=("target/release/lantern")
  else
    rustup target list --installed 2>/dev/null | grep -qx "$t" || ui_run "Rust-Ziel $t (einmalig)" rustup target add "$t"
    ui_run "Programm für $a" cargo build --release --locked --target "$t"
    BINS+=("target/$t/release/lantern")
  fi
done

# FFmpeg for the videos, one per architecture, joined like the program.
FF=target/ffmpeg/mac
fetch_ffmpeg() {
  rm -rf "$FF" && mkdir -p "$FF"
  local parts=()
  for a in $ARCHS; do
    case "$a" in arm64|aarch64) a=arm64 ;; *) a=x86_64 ;; esac
    packaging/fetch-ffmpeg.sh "macos-$a" "$FF/$a"
    parts+=("$FF/$a/ffmpeg")
  done
  lipo -create -output "$FF/ffmpeg" "${parts[@]}"
  cp packaging/FFMPEG.txt "$FF/FFMPEG.txt"
}
ui_run "FFmpeg für Videos" fetch_ffmpeg

DMG="dist/Lantern-$V.dmg"
KEY="$V-$(ui_fingerprint "${BINS[@]}" "$FF/ffmpeg" packaging/macos/Info.plist assets/icon.png)"
make_dmg() {
  WORK=$(mktemp -d)
  APP="$WORK/dmg/Lantern.app"
  mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
  lipo -create -output "$APP/Contents/MacOS/lantern" "${BINS[@]}"
  cp "$FF/ffmpeg" "$APP/Contents/MacOS/ffmpeg"
  cp "$FF/FFMPEG.txt" "$APP/Contents/Resources/FFMPEG.txt"
  sed "s/APP_VERSION/$V/g" packaging/macos/Info.plist > "$APP/Contents/Info.plist"
  ICONSET="$WORK/Lantern.iconset"
  mkdir -p "$ICONSET"
  for s in 16 32 128 256 512; do
    sips -z $s $s assets/icon.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
    sips -z $((s*2)) $((s*2)) assets/icon.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
  done
  iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/Lantern.icns"
  # Ad-hoc signature: macOS asks once for "Open Anyway".
  codesign --force --deep -s - "$APP"
  ln -s /Applications "$WORK/dmg/Applications"
  rm -f "$DMG"
  hdiutil create -volname "Lantern" -srcfolder "$WORK/dmg" -ov -format UDZO "$DMG"
  rm -rf "$WORK"
}
if ui_fresh "$DMG" "$KEY"; then ui_skip "App und DMG" "unverändert"; else
  ui_run "App und DMG" make_dmg && ui_made "$DMG" "$KEY"
fi
{ echo "$V"; echo "$DMG"; } > dist/files-mac.txt
ui_done
