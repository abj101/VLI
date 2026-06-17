# Downloads bundled Whisper ggml weights into src-tauri/resources (not committed).
param(
    [ValidateSet("tiny.en", "base.en", "small.en")]
    [string]$Model = "tiny.en"
)

$ErrorActionPreference = "Stop"
$dest = Join-Path $PSScriptRoot "..\src-tauri\resources\ggml-$Model.bin"
$url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-$Model.bin"
New-Item -ItemType Directory -Force (Split-Path $dest) | Out-Null
Write-Host "Downloading $url"
Invoke-WebRequest -Uri $url -OutFile $dest
Write-Host "Wrote $dest"
