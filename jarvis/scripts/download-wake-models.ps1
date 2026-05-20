# OpenWakeWord ONNX models — delegates to the npm script (same files as `prebuild`).
# Usage (from repo root):  .\jarvis\scripts\download-wake-models.ps1
# Or:                       cd jarvis; npm run fetch-wake-models

$ErrorActionPreference = "Stop"
$JarvisRoot = Split-Path -Parent $PSScriptRoot
Push-Location $JarvisRoot
try {
  npm run fetch-wake-models
} finally {
  Pop-Location
}
