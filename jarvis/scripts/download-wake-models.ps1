# Downloads Porcupine runtime + keyword model for local dev (binaries are gitignored).
# Files are fetched from the public Picovoice GitHub repo (no API key).
# Picovoice Console access key is only needed at runtime (keychain); see Settings in T4-4.
#
# Usage:
#   .\scripts\download-wake-models.ps1
#
# Produces under src-tauri/resources/porcupine/:
#   - libpv_porcupine.dll (Windows x64; Picovoice ships this name)
#   - porcupine_params.pv
#   - porcupine_windows.ppn (built-in keyword; swap for jarvis_windows.ppn from Picovoice Console for "jarvis")
#
# Version pins should match the Picovoice Porcupine release you target.

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
$Tauri = Join-Path $Root "src-tauri"
$Dest = Join-Path $Tauri "resources\porcupine"
New-Item -ItemType Directory -Force -Path $Dest | Out-Null

# Picovoice public repo paths (same layout as Porcupine C demo).
$Base = "https://raw.githubusercontent.com/Picovoice/porcupine/master"
$LibUrl = "$Base/lib/windows/amd64/libpv_porcupine.dll"
$PvUrl = "$Base/lib/common/porcupine_params.pv"
$PpnUrl = "$Base/resources/keyword_files/windows/porcupine_windows.ppn"

Write-Host "Downloading Porcupine libs into $Dest ..."
Invoke-WebRequest -Uri $LibUrl -OutFile (Join-Path $Dest "libpv_porcupine.dll")
Invoke-WebRequest -Uri $PvUrl -OutFile (Join-Path $Dest "porcupine_params.pv")
Invoke-WebRequest -Uri $PpnUrl -OutFile (Join-Path $Dest "porcupine_windows.ppn")

Write-Host "Done. Store your Picovoice access key in OS keychain via JARVIS Settings (T4-4) or keyring service jarvis-porcupine."
