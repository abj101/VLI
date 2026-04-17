import { describe, expect, it } from "vitest";

import {
  buildWingetInstallArgs,
  formatVulkanSdkTauriCmdBody,
  quoteBatchArgWindows,
  quoteBatchPathWindows,
} from "./tauri-whisper-gpu-launch.mjs";

describe("quoteBatchPathWindows", () => {
  it("doubles internal quotes for cmd.exe rules", () => {
    expect(quoteBatchPathWindows(`C:\\a"b\\c`)).toBe(`"C:\\a""b\\c"`);
  });

  it("wraps Program Files node path", () => {
    const p = "C:\\Program Files\\nodejs\\node.exe";
    expect(quoteBatchPathWindows(p)).toBe(`"${p}"`);
  });
});

describe("quoteBatchArgWindows", () => {
  it("leaves whisper-vulkan bare", () => {
    expect(quoteBatchArgWindows("whisper-vulkan")).toBe("whisper-vulkan");
  });

  it("quotes args with spaces", () => {
    expect(quoteBatchArgWindows("a b")).toBe(`"a b"`);
  });
});

describe("formatVulkanSdkTauriCmdBody", () => {
  it("produces set VULKAN_SDK then quoted node and tauri paths (Program Files safe)", () => {
    const body = formatVulkanSdkTauriCmdBody({
      vkRoot: "C:\\VulkanSDK\\1.4.341.1",
      nodeExe: "C:\\Program Files\\nodejs\\node.exe",
      tauriCli: "D:\\repo\\jarvis\\node_modules\\@tauri-apps\\cli\\tauri.js",
      args: ["dev", "--features", "whisper-vulkan"],
    });
    expect(body.startsWith("@echo off\r\n")).toBe(true);
    expect(body).toContain('set "VULKAN_SDK=C:\\VulkanSDK\\1.4.341.1"');
    expect(body).toContain('"C:\\Program Files\\nodejs\\node.exe"');
    expect(body).toContain('"D:\\repo\\jarvis\\node_modules\\@tauri-apps\\cli\\tauri.js"');
    expect(body).toMatch(/dev.*whisper-vulkan/);
    expect(body).not.toMatch(/nodejs\\node\.exe\\/);
  });

  it("escapes percent in VULKAN_SDK for batch", () => {
    const body = formatVulkanSdkTauriCmdBody({
      vkRoot: "C:\\Vulkan%SDK%",
      nodeExe: "C:\\node.exe",
      tauriCli: "C:\\tauri.js",
      args: ["build"],
    });
    expect(body).toContain('set "VULKAN_SDK=C:\\Vulkan%%SDK%%"');
  });
});

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
