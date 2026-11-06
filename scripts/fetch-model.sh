#!/bin/sh
# Fetches the speech models bundled into Otto (not in git) into models/ and verifies them:
# Parakeet TDT 0.6B v2 (Q8_0, ~730 MB, the default speech model) and Silero VAD v6.2.
# A copy already in Otto's user models folder is reused instead of downloading.
set -eu
cd "$(dirname "$0")/.."
mkdir -p models

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

# fetch <file> <url> <sha256>
fetch() {
  dest="models/$1"
  if [ -s "$dest" ]; then
    [ "$(sha256 "$dest")" = "$3" ] && return 0
    echo "fetch-model: $dest has the wrong checksum, fetching again" >&2
    rm -f "$dest"
  fi
  for dir in "$HOME/Library/Application Support/app.otto.desktop/models" "${APPDATA:-/nonexistent}/app.otto.desktop/models"; do
    if [ -s "$dir/$1" ] && [ "$(sha256 "$dir/$1")" = "$3" ]; then
      cp "$dir/$1" "$dest"
      return 0
    fi
  done
  curl -fL --retry 3 -o "$dest.part" "$2"
  got=$(sha256 "$dest.part")
  if [ "$got" != "$3" ]; then
    rm -f "$dest.part"
    echo "fetch-model: checksum mismatch for $1: got $got, want $3" >&2
    exit 1
  fi
  mv "$dest.part" "$dest"
}

fetch parakeet-tdt-0.6b-v2-Q8_0.gguf \
  https://huggingface.co/handy-computer/parakeet-tdt-0.6b-v2-gguf/resolve/main/parakeet-tdt-0.6b-v2-Q8_0.gguf \
  f0d0e99cebb6d3b83f1f7069b82b5d3c2e39a54545b0da039cb4bafd9c4e5caa
fetch silero-vad-v6.2-F32.gguf \
  https://huggingface.co/handy-computer/silero-vad-v6.2-gguf/resolve/main/silero-vad-v6.2-F32.gguf \
  0d871a800c420fdd5d1b17a3aea4296d1c39608f0ff4a9295eb76457eb1c88e6
