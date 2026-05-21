/**
 * Windows launch helpers (PATH merge, stale jarvis.exe release).
 * Winget helpers remain for tests; SDK install is manual (see jarvis/README.md).
 */

export function buildWingetInstallArgs(packageId, opts = {}) {
  const { disableInteractivity = false } = opts;
  const a = [
    "install",
    "-e",
    "--id",
    packageId,
    "--accept-package-agreements",
    "--accept-source-agreements",
  ];
  if (disableInteractivity) {
    a.push("--disable-interactivity");
  }
  return a;
}

const WINGET_UPDATE_NOT_APPLICABLE = 0x8a15002b;

export function isWingetInstallSuccessStatus(status) {
  if (status === 0) return true;
  if (!Number.isInteger(status)) return false;
  return (status >>> 0) === WINGET_UPDATE_NOT_APPLICABLE;
}

export function shouldReleaseWindowsJarvisExeLockForSubcommand(subcommand) {
  return subcommand === "dev" || subcommand === "build";
}

export function prependWindowsPathEntries(pathValue, entries) {
  const parts = String(pathValue ?? "")
    .split(";")
    .map((p) => p.trim())
    .filter(Boolean);
  const seen = new Set(parts.map((p) => p.toLowerCase()));
  const prepend = [];
  for (const raw of entries ?? []) {
    const p = String(raw ?? "").trim();
    if (!p) continue;
    const key = p.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    prepend.push(p);
  }
  return [...prepend, ...parts].join(";");
}

export function buildWindowsTerminateByExecutablePathScript(exePath) {
  const target = String(exePath).replace(/'/g, "''");
  return [
    `$target = '${target}'`,
    "$killed = 0",
    "Get-CimInstance Win32_Process -Filter \"Name = 'jarvis.exe'\" |",
    "  Where-Object { $_.ExecutablePath -and ($_.ExecutablePath -ieq $target) } |",
    "  ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue; $killed += 1 }",
    "Write-Output $killed",
  ].join("; ");
}
