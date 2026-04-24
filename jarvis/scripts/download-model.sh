#!/usr/bin/env bash
set -euo pipefail

# Downloads bundled Whisper tiny.en weights into src-tauri/resources (not committed).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEST="${SCRIPT_DIR}/../src-tauri/resources/ggml-tiny.en.bin"
URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin"

mkdir -p "$(dirname "${DEST}")"
echo "Downloading ${URL}"
curl -fL "${URL}" -o "${DEST}"
echo "Wrote ${DEST}"
