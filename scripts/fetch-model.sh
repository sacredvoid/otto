#!/bin/sh
# Fetches the Whisper speech model bundled into Otto.app (148 MB, not in git).
set -eu
cd "$(dirname "$0")/.."
dest=models/ggml-base.en.bin
[ -s "$dest" ] && exit 0
mkdir -p models
local="$HOME/Library/Application Support/app.otto.desktop/models/ggml-base.en.bin"
if [ -s "$local" ]; then cp "$local" "$dest"; exit 0; fi
curl -fL --retry 3 -o "$dest.part" https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin
mv "$dest.part" "$dest"
