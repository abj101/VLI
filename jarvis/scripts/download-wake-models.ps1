# Downloads Porcupine runtime + keyword model for local dev (binaries are gitignored).
# Requires: Picovoice Console access key — https://console.picovoice.ai/
#
# Usage:
#   $env:PICOVOICE_ACCESS_KEY = "..."   # or pass -AccessKey
#   .\scripts\download-wake-models.ps1
#
# Produces under src-tauri/resources/porcupine/:
#   - libpv_porcupine.dll (Windows x64; Picovoice ships this name)
#   - porcupine_params.pv
#   - porcupine_windows.ppn (built-in keyword; swap for jarvis_windows.ppn from Picovoice Console for "jarvis")
#
# Version pins should match the Picovoice Porcupine release you target.

param(
    [string] $AccessKey = $env:PICOVOICE_ACCESS_KEY
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
$Tauri = Join-Path $Root "src-tauri"
$Dest = Join-Path $Tauri "resources\porcupine"
New-Item -ItemType Directory -Force -Path $Dest | Out-Null

if (-not $AccessKey) {
    Write-Warning "Set PICOVOICE_ACCESS_KEY or pass -AccessKey. Then re-run to download libs."
    Write-Host "See: https://console.picovoice.ai/ and jarvis/README.md (Phase 4)."
    exit 1
}

# Picovoice public repo paths (same layout as Porcupine C demo).
$Base = "https://raw.githubusercontent.com/Picovoice/porcupine/master"
$LibUrl = "$Base/lib/windows/amd64/libpv_porcupine.dll"
$PvUrl = "$Base/lib/common/porcupine_params.pv"
$PpnUrl = "$Base/resources/keyword_files/windows/porcupine_windows.ppn"

Write-Host "Downloading Porcupine libs into $Dest ..."
Invoke-WebRequest -Uri $LibUrl -OutFile (Join-Path $Dest "libpv_porcupine.dll")
Invoke-WebRequest -Uri $PvUrl -OutFile (Join-Path $Dest "porcupine_params.pv")
Invoke-WebRequest -Uri $PpnUrl -OutFile (Join-Path $Dest "porcupine_windows.ppn")

Write-Host "Done. Store your access key in OS keychain via JARVIS Settings (T4-4) or keyring service jarvis-porcupine."
