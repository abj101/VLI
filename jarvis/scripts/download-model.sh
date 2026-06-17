#!/usr/bin/env bash
set -euo pipefail

# Downloads bundled Whisper ggml weights into src-tauri/resources (not committed).
MODEL="${1:-tiny.en}"
case "${MODEL}" in
  tiny.en|base.en|small.en) ;;
  *)
    echo "Usage: $0 [tiny.en|base.en|small.en]" >&2
    exit 1
    ;;
esac

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEST="${SCRIPT_DIR}/../src-tauri/resources/ggml-${MODEL}.bin"
URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-${MODEL}.bin"

mkdir -p "$(dirname "${DEST}")"
echo "Downloading ${URL}"
curl -fL "${URL}" -o "${DEST}"
echo "Wrote ${DEST}"
