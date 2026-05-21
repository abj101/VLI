import path from "node:path";
import { describe, expect, it } from "vitest";

import {
  normalizeWindowsVulkanSdkRoot,
  windowsVulkanSdkLayoutOk,
} from "./whisper-gpu/win-sdk.mjs";

describe("windowsVulkanSdkLayoutOk", () => {
  it("returns true when Include and Lib exist", () => {
    const root = path.resolve(".vitest-fixtures", "vk-sdk", "1.0.0");
    const inc = path.join(root, "Include");
    const lib = path.join(root, "Lib");
    const exists = (p) => p === inc || p === lib;
    expect(windowsVulkanSdkLayoutOk(root, exists)).toBe(true);
  });

  it("returns false when Lib missing", () => {
    const root = path.resolve(".vitest-fixtures", "vk-sdk", "1.0.0");
    const exists = (p) => p === path.join(root, "Include");
    expect(windowsVulkanSdkLayoutOk(root, exists)).toBe(false);
  });
});

describe("normalizeWindowsVulkanSdkRoot", () => {
  it("walks up from Bin to SDK root", () => {
    const root = path.resolve(".vitest-fixtures", "vk-sdk", "1.2.3");
    const inc = path.join(root, "Include");
    const lib = path.join(root, "Lib");
    const roots = new Set([inc, lib]);
    const exists = (p) => roots.has(p);
    const bin = path.join(root, "Bin");
    expect(normalizeWindowsVulkanSdkRoot(bin, exists)).toBe(root);
  });
});
