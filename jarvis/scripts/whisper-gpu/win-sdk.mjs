/**
 * Windows Vulkan SDK path helpers.
 */

import fs from "fs";
import path from "path";

export function windowsVulkanSdkLayoutOk(sdkRoot, existsSync = fs.existsSync) {
  if (!sdkRoot) return false;
  const includeDir = path.join(sdkRoot, "Include");
  const libDir = path.join(sdkRoot, "Lib");
  return existsSync(includeDir) && existsSync(libDir);
}

export function normalizeWindowsVulkanSdkRoot(raw, existsSync = fs.existsSync) {
  if (!raw || typeof raw !== "string") return null;
  let cur = path.resolve(raw.trim());
  for (let depth = 0; depth < 8; depth += 1) {
    if (windowsVulkanSdkLayoutOk(cur, existsSync)) {
      return cur;
    }
    const parent = path.dirname(cur);
    if (parent === cur) {
      break;
    }
    cur = parent;
  }
  return null;
}
