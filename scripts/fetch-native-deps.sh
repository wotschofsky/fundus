#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
mkdir -p .cache/native
archive=.cache/native/flac-1.5.0.tar.xz
if [ ! -f "$archive" ]; then curl -fL https://downloads.xiph.org/releases/flac/flac-1.5.0.tar.xz -o "$archive"; fi
printf '%s  %s\n' f2c1c76592a82ffff8413ba3c4a1299b6c7ab06c734dee03fd88630485c2b920 "$archive" | shasum -a 256 -c -
if [ ! -d .cache/native/flac-1.5.0 ]; then tar -xf "$archive" -C .cache/native; fi
