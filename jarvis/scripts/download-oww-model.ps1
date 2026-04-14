# Downloads OpenWakeWord ONNX models for the optional `oww` Rust feature (T4-2).
# Models are Apache-2.0 (see https://github.com/dscripka/openWakeWord ); large binaries stay gitignored.
#
# Usage:
#   .\scripts\download-oww-model.ps1
#   (or from jarvis/: `npm run fetch-wake-models` — same files via Node)
#
# Produces under src-tauri/resources/oww/:
#   - melspectrogram.onnx
#   - embedding_model.onnx
#   - hey_jarvis_v0.1.onnx
#
# Build with: cargo build --features oww --no-default-features   (or add `oww` alongside default features)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
$Tauri = Join-Path $Root "src-tauri"
$Dest = Join-Path $Tauri "resources\oww"
New-Item -ItemType Directory -Force -Path $Dest | Out-Null

$Release = "https://github.com/dscripka/openWakeWord/releases/download/v0.5.1"
$Files = @(
    "melspectrogram.onnx",
    "embedding_model.onnx",
    "hey_jarvis_v0.1.onnx"
)

Write-Host "Downloading OpenWakeWord ONNX models into $Dest ..."
foreach ($f in $Files) {
    $url = "$Release/$f"
    $out = Join-Path $Dest $f
    Write-Host "  $f"
    Invoke-WebRequest -Uri $url -OutFile $out
}
Write-Host "Done. Set JARVIS_OWW_MODEL_DIR to $Tauri\resources for optional `cargo test --features oww` integration tests."
