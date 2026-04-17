$ErrorActionPreference = "Stop"

$subcommand = if ($args.Length -gt 0) { $args[0] } else { "build" }
$extra = if ($args.Length -gt 1) { $args[1..($args.Length - 1)] } else { @() }

if ($subcommand -ne "build" -and $subcommand -ne "dev") {
  throw "First arg must be 'build' or 'dev'."
}

$script = Join-Path $PSScriptRoot "tauri-whisper-gpu.mjs"
node $script $subcommand @extra
