#!/bin/sh
set -eu
model_dir="${VAULT_MODEL_DIR:-.cache/models}"
mkdir -p "$model_dir"
file="$model_dir/speaker-embedding.onnx"
expected=7bb2f06e9df17cdf1ef14ee8a15ab08ed28e8d0ef5054ee135741560df2ec068
checksum() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d ' ' -f 1; else shasum -a 256 "$1" | cut -d ' ' -f 1; fi; }
if [ ! -f "$file" ] || [ "$(checksum "$file")" != "$expected" ]; then
 curl --fail --location --retry 3 https://huggingface.co/hbredin/wespeaker-voxceleb-resnet34-LM/resolve/0ae88dcaf48cacdf741275d6d1a8101f45eee220/speaker-embedding.onnx -o "$file.partial"
 [ "$(checksum "$file.partial")" = "$expected" ] || { rm -f "$file.partial"; echo 'Speaker model checksum mismatch' >&2; exit 1; }
 mv "$file.partial" "$file"
fi
printf '%s\n' "$file"
