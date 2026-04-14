# Downloads bundled Whisper tiny.en weights into src-tauri/resources (not committed).
$ErrorActionPreference = "Stop"
$dest = Join-Path $PSScriptRoot "..\src-tauri\resources\ggml-tiny.en.bin"
$url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin"
New-Item -ItemType Directory -Force (Split-Path $dest) | Out-Null
Write-Host "Downloading $url"
Invoke-WebRequest -Uri $url -OutFile $dest
Write-Host "Wrote $dest"
