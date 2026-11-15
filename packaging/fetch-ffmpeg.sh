#!/bin/bash
# Puts a stand-alone FFmpeg for one platform into a folder, for the packages:
#   packaging/fetch-ffmpeg.sh <macos-arm64|macos-x86_64|linux-x86_64|windows-x86_64> <folder>
# Lantern starts it as a separate program to play the videos in presentations.
# Downloads are kept in target/ffmpeg/ and checked against their checksums.
#   macOS:           https://ffmpeg.martin-riedl.de (static, GPL, pinned below)
#   Linux, Windows:  https://github.com/BtbN/FFmpeg-Builds (static, LGPL, branch n9.0)
set -euo pipefail
cd "$(dirname "$0")/.."
PLATFORM="$1"
OUT="$2"
CACHE="target/ffmpeg/$PLATFORM"
mkdir -p "$CACHE" "$OUT"

sha256() { if command -v sha256sum >/dev/null; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1; }
fetch() { # url file
  [ -s "$2" ] || { curl -fsSL --retry 3 -o "$2.part" "$1" && mv "$2.part" "$2"; }
}
check() { # file sha
  local got; got=$(sha256 "$1")
  [ "$got" = "$2" ] || { rm -f "$1"; echo "Checksum of $1 is wrong ($got)" >&2; exit 1; }
}

MR=https://ffmpeg.martin-riedl.de/download/macos
BTBN=https://github.com/BtbN/FFmpeg-Builds/releases/download/latest
case "$PLATFORM" in
  macos-arm64|macos-x86_64)
    if [ "$PLATFORM" = macos-arm64 ]; then
      URL="$MR/arm64/1789931890_9.0.2/ffmpeg.zip"
      SHA=c8ed4c4e6978a03c485edbfe4e0a5dc2380f8a30bba5150531b31b094492d924
    else
      URL="$MR/amd64/1789931006_9.0.2/ffmpeg.zip"
      SHA=7c6b4125b191cbf773832dc51f424cf2b6bb7da43007d1e066f95909e47cacd4
    fi
    fetch "$URL" "$CACHE/ffmpeg.zip"
    check "$CACHE/ffmpeg.zip" "$SHA"
    unzip -o -q "$CACHE/ffmpeg.zip" ffmpeg -d "$OUT"
    chmod +x "$OUT/ffmpeg"
    ;;
  linux-x86_64|windows-x86_64)
    if [ "$PLATFORM" = linux-x86_64 ]; then
      NAME=ffmpeg-n9.0-latest-linux64-lgpl-9.0.tar.xz
    else
      NAME=ffmpeg-n9.0-latest-win64-lgpl-9.0.zip
    fi
    # The "latest" builds change daily: always fetch the checksums, re-download if needed.
    curl -fsSL --retry 3 -o "$CACHE/checksums.sha256" "$BTBN/checksums.sha256"
    SHA=$(grep " \*\?$NAME\$" "$CACHE/checksums.sha256" | cut -d' ' -f1)
    [ -n "$SHA" ] || { echo "$NAME is not in the checksums" >&2; exit 1; }
    if [ ! -s "$CACHE/$NAME" ] || [ "$(sha256 "$CACHE/$NAME")" != "$SHA" ]; then
      rm -f "$CACHE/$NAME"
      fetch "$BTBN/$NAME" "$CACHE/$NAME"
    fi
    check "$CACHE/$NAME" "$SHA"
    DIR="${NAME%.tar.xz}"; DIR="${DIR%.zip}"
    if [ "$PLATFORM" = linux-x86_64 ]; then
      tar -xJf "$CACHE/$NAME" -C "$CACHE" "$DIR/bin/ffmpeg"
      cp "$CACHE/$DIR/bin/ffmpeg" "$OUT/ffmpeg" && chmod +x "$OUT/ffmpeg"
    else
      unzip -o -q -j "$CACHE/$NAME" "$DIR/bin/ffmpeg.exe" -d "$OUT"
    fi
    ;;
  *) echo "Unknown platform: $PLATFORM" >&2; exit 1 ;;
esac
cp packaging/FFMPEG.txt "$OUT/FFMPEG.txt"
