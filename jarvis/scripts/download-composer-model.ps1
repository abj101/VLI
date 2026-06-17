# Downloads the command composer GGUF into src-tauri/resources (not committed).
$ErrorActionPreference = "Stop"
Push-Location (Join-Path $PSScriptRoot "..")
try {
  npm run fetch-llm-models
} finally {
  Pop-Location
}
