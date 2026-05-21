import { describe, expect, it } from "vitest";

import {
  buildWindowsTerminateByExecutablePathScript,
  buildWingetInstallArgs,
  isWingetInstallSuccessStatus,
  prependWindowsPathEntries,
  shouldReleaseWindowsJarvisExeLockForSubcommand,
} from "./whisper-gpu/launch.mjs";

describe("buildWingetInstallArgs", () => {
  it("includes core flags", () => {
    expect(buildWingetInstallArgs("KhronosGroup.VulkanSDK")).toEqual([
      "install",
      "-e",
      "--id",
      "KhronosGroup.VulkanSDK",
      "--accept-package-agreements",
      "--accept-source-agreements",
    ]);
  });

  it("optionally adds disable-interactivity", () => {
    expect(
      buildWingetInstallArgs("Nvidia.CUDA", { disableInteractivity: true }),
    ).toContain("--disable-interactivity");
  });
});

describe("isWingetInstallSuccessStatus", () => {
  it("treats winget UPDATE_NOT_APPLICABLE as success for install", () => {
    expect(isWingetInstallSuccessStatus(2316632107)).toBe(true);
  });
});

describe("buildWindowsTerminateByExecutablePathScript", () => {
  it("escapes single quotes and includes process kill pipeline", () => {
    const script = buildWindowsTerminateByExecutablePathScript(
      "C:\\repo\\jarvis\\src-tauri\\target\\debug\\jarvis's.exe",
    );
    expect(script).toContain("$killed = 0");
    expect(script).toContain("Get-CimInstance Win32_Process");
    expect(script).toContain("Stop-Process -Id $_.ProcessId -Force");
    expect(script).toContain("jarvis''s.exe");
  });
});

describe("shouldReleaseWindowsJarvisExeLockForSubcommand", () => {
  it("releases stale exe lock for dev + build only", () => {
    expect(shouldReleaseWindowsJarvisExeLockForSubcommand("dev")).toBe(true);
    expect(shouldReleaseWindowsJarvisExeLockForSubcommand("build")).toBe(true);
    expect(shouldReleaseWindowsJarvisExeLockForSubcommand("info")).toBe(false);
  });
});

describe("prependWindowsPathEntries", () => {
  it("prepends missing entries and dedups case-insensitively", () => {
    const base = "C:\\Windows\\System32;C:\\Tools";
    const merged = prependWindowsPathEntries(base, [
      "C:\\CUDA\\bin",
      "c:\\tools",
      "  ",
      null,
    ]);
    expect(merged).toBe("C:\\CUDA\\bin;C:\\Windows\\System32;C:\\Tools");
  });
});
