/**
 * Windows `.cmd` launcher for Tauri when `VULKAN_SDK` must be visible to nested Cargo.
 * Avoids fragile `cmd /c "set ...&& node \"...\""` quoting (breaks under `Program Files\...`).
 */

/** Batch: wrap path in quotes; internal `"` → `""`. */
export function quoteBatchPathWindows(p) {
  const s = String(p);
  return `"${s.replace(/"/g, '""')}"`;
}

/** Quote arg only if cmd metacharacters or spaces appear. */
export function quoteBatchArgWindows(a) {
  const s = String(a);
  if (/[\s^&|%<>()]/.test(s)) {
    return quoteBatchPathWindows(s);
  }
  return s;
}

/**
 * Returns a `.cmd` file body: set VULKAN_SDK, then `node` + `tauri.js` + args (each safely quoted).
 */
export function formatVulkanSdkTauriCmdBody({ vkRoot, nodeExe, tauriCli, args }) {
  const safeVk = String(vkRoot).replace(/%/g, "%%");
  const parts = [
    quoteBatchPathWindows(nodeExe),
    quoteBatchPathWindows(tauriCli),
    ...args.map(quoteBatchArgWindows),
  ];
  return ["@echo off", `set "VULKAN_SDK=${safeVk}"`, parts.join(" ")].join("\r\n");
}

/**
 * winget CLI args for SDK installs. `--disable-interactivity` is appended only when supported
 * (older winget errors on unknown flags).
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

// `winget install` can return UPDATE_NOT_APPLICABLE when package is already current.
const WINGET_UPDATE_NOT_APPLICABLE = 0x8a15002b;

export function isWingetInstallSuccessStatus(status) {
  if (status === 0) return true;
  if (!Number.isInteger(status)) return false;
  return (status >>> 0) === WINGET_UPDATE_NOT_APPLICABLE;
}
