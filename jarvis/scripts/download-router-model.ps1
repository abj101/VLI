# Downloads the default on-device router GGUF into src-tauri/resources (not committed).
$ErrorActionPreference = "Stop"
$dest = Join-Path $PSScriptRoot "..\src-tauri\resources\qwen2.5-0.5b-instruct-q4_k_m.gguf"
$url = "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf"
New-Item -ItemType Directory -Force (Split-Path $dest) | Out-Null
Write-Host "Downloading $url"
Invoke-WebRequest -Uri $url -OutFile $dest
Write-Host "Wrote $dest"
