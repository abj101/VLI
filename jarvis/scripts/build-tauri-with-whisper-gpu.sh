#!/usr/bin/env bash
set -euo pipefail

subcommand="${1:-build}"
shift || true

if [[ "$subcommand" != "build" && "$subcommand" != "dev" ]]; then
  echo "First arg must be 'build' or 'dev'." >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
node "$SCRIPT_DIR/whisper-gpu/run-tauri.mjs" "$subcommand" "$@"
