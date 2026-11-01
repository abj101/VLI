# Downloads the default on-device d1 router GGUF into src-tauri/resources (not committed).
$ErrorActionPreference = "Stop"
Push-Location (Join-Path $PSScriptRoot "..")
try {
  npm run fetch-llm-models
} finally {
  Pop-Location
}
