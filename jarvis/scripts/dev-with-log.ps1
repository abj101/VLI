# Run `tauri dev` with backend debug logs mirrored to jarvis-dev.log (repo root of jarvis/).
$env:RUST_LOG = 'info,jarvis_lib=debug'
Set-Location (Split-Path $PSScriptRoot -Parent)
npm run tauri:dev 2>&1 | Tee-Object -FilePath jarvis-dev.log
