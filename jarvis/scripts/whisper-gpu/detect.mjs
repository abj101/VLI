/**
 * GPU vendor detection, CUDA/Vulkan toolchain probes, Whisper backend auto-select.
 */

import fs from "fs";
import path from "path";
import { spawnSync } from "child_process";

import {
  compareVersionNamesDesc,
  discoverWindowsMsvcHostX64BinDir,
} from "./win-env.mjs";
import {
  normalizeWindowsVulkanSdkRoot,
  windowsVulkanSdkLayoutOk,
} from "./win-sdk.mjs";

const VALID_BACKENDS = new Set(["metal", "cuda", "vulkan", "none"]);

export function run(cmd, args) {
  const out = spawnSync(cmd, args, { encoding: "utf8" });
  return {
    ok: out.status === 0,
    stdout: out.stdout ?? "",
    stderr: out.stderr ?? "",
  };
}

export function commandExists(cmd, args = ["--version"]) {
  return run(cmd, args).ok;
}

function hasPath(dir) {
  return typeof dir === "string" && dir.trim().length > 0 && fs.existsSync(dir);
}

function windowsCudaToolkitLayoutOk(cudaRoot) {
  if (!cudaRoot) return false;
  const nvcc = path.join(cudaRoot, "bin", "nvcc.exe");
  const includeCuda = path.join(cudaRoot, "include", "cuda.h");
  return fs.existsSync(nvcc) && fs.existsSync(includeCuda);
}

function classifyVendor(text) {
  if (/nvidia/i.test(text)) return "nvidia";
  if (/amd|radeon/i.test(text)) return "amd";
  if (/intel/i.test(text)) return "intel";
  if (/apple/i.test(text)) return "apple";
  return "unknown";
}

export function detectGpuVendor() {
  if (commandExists("nvidia-smi", ["-L"])) return "nvidia";

  if (process.platform === "win32") {
    const probe = run("wmic", ["path", "win32_VideoController", "get", "Name"]);
    if (!probe.ok) return "unknown";
    return classifyVendor(probe.stdout);
  }
  if (process.platform === "linux") {
    const probe = run("lspci", []);
    if (!probe.ok) return "unknown";
    return classifyVendor(probe.stdout);
  }
  if (process.platform === "darwin") {
    const probe = run("system_profiler", ["SPDisplaysDataType"]);
    if (!probe.ok) return "unknown";
    return classifyVendor(probe.stdout);
  }
  return "unknown";
}

function discoverWindowsCudaToolkitRoot() {
  const envCandidates = [];
  if (hasPath(process.env.CUDA_PATH)) {
    envCandidates.push(process.env.CUDA_PATH);
  }
  for (const [k, v] of Object.entries(process.env)) {
    if (!/^CUDA_PATH_V/i.test(k)) continue;
    if (hasPath(v)) envCandidates.push(v);
  }
  for (const c of envCandidates) {
    if (windowsCudaToolkitLayoutOk(c)) {
      return path.resolve(c.trim());
    }
  }

  const installRoot = "C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA";
  if (!fs.existsSync(installRoot)) {
    return null;
  }
  const dirs = fs
    .readdirSync(installRoot, { withFileTypes: true })
    .filter((d) => d.isDirectory())
    .map((d) => path.join(installRoot, d.name))
    .filter((p) => windowsCudaToolkitLayoutOk(p))
    .sort((a, b) => compareVersionNamesDesc(path.basename(a), path.basename(b)));
  return dirs[0] ?? null;
}

export function resolveWindowsCudaToolkitRoot() {
  const fromEnv = process.env.CUDA_PATH;
  if (hasPath(fromEnv) && windowsCudaToolkitLayoutOk(fromEnv)) {
    return path.resolve(fromEnv.trim());
  }
  return discoverWindowsCudaToolkitRoot();
}

export function applyDiscoveredCudaToolkitToProcessEnv() {
  if (process.platform !== "win32") return null;
  const resolved = resolveWindowsCudaToolkitRoot();
  if (resolved) {
    process.env.CUDA_PATH = resolved;
    return resolved;
  }
  return null;
}

export function hasCudaToolchain() {
  if (process.platform === "win32") {
    return !!resolveWindowsCudaToolkitRoot() || commandExists("nvcc");
  }
  const cudaPath = process.env.CUDA_PATH;
  if (hasPath(cudaPath)) return true;
  return commandExists("nvcc");
}

function appendWindowsClFlag(envObj, flag) {
  const token = flag.trim();
  if (!token) return;
  for (const key of ["CL", "_CL_"]) {
    const prev = envObj[key]?.trim();
    if (prev?.includes(token)) continue;
    envObj[key] = prev ? `${prev} ${token}` : token;
  }
}

export function applyWindowsCudaBuildEnvIfNeeded(envObj) {
  if (process.platform !== "win32") return null;
  const cudaRoot = resolveWindowsCudaToolkitRoot();
  if (!cudaRoot) return null;

  envObj.CUDA_PATH = envObj.CUDA_PATH || cudaRoot;

  const nvccExe = path.join(cudaRoot, "bin", "nvcc.exe");
  if (!fs.existsSync(nvccExe)) return null;

  const msvcHostBin = discoverWindowsMsvcHostX64BinDir();
  if (!msvcHostBin) return null;
  const nmakeExe = path.join(msvcHostBin, "nmake.exe");
  const clExe = path.join(msvcHostBin, "cl.exe");
  if (!fs.existsSync(nmakeExe) || !fs.existsSync(clExe)) return null;

  if (!envObj.CMAKE_GENERATOR) envObj.CMAKE_GENERATOR = "NMake Makefiles";
  if (envObj.CMAKE_GENERATOR === "NMake Makefiles") {
    delete envObj.CMAKE_GENERATOR_INSTANCE;
    delete envObj.CMAKE_GENERATOR_PLATFORM;
    delete envObj.CMAKE_GENERATOR_TOOLSET;
  }
  if (!envObj.CMAKE_MAKE_PROGRAM) envObj.CMAKE_MAKE_PROGRAM = nmakeExe;
  if (!envObj.CMAKE_CUDA_COMPILER) envObj.CMAKE_CUDA_COMPILER = nvccExe;
  if (!envObj.CMAKE_CUDA_HOST_COMPILER) envObj.CMAKE_CUDA_HOST_COMPILER = clExe;
  if (!envObj.CMAKE_CUDA_FLAGS) envObj.CMAKE_CUDA_FLAGS = "-Xcompiler=/Zc:preprocessor";
  appendWindowsClFlag(envObj, "/Zc:preprocessor");
  if (!envObj.CMAKE_SUPPRESS_DEVELOPER_WARNINGS) {
    envObj.CMAKE_SUPPRESS_DEVELOPER_WARNINGS = "1";
  }
  return {
    cudaRoot,
    generator: envObj.CMAKE_GENERATOR,
    nmakeExe,
  };
}

function discoverWindowsVulkanSdk() {
  const candidates = [
    "C:\\VulkanSDK",
    "C:\\Program Files\\VulkanSDK",
    "C:\\Program Files (x86)\\VulkanSDK",
  ];
  for (const root of candidates) {
    if (!fs.existsSync(root)) continue;
    const dirs = fs
      .readdirSync(root, { withFileTypes: true })
      .filter((d) => d.isDirectory())
      .map((d) => d.name)
      .sort(compareVersionNamesDesc);
    for (const dirName of dirs) {
      const sdkPath = path.join(root, dirName);
      if (windowsVulkanSdkLayoutOk(sdkPath)) {
        return sdkPath;
      }
    }
  }
  return null;
}

function sdkRootFromVulkaninfoExe(exePath) {
  const norm = exePath.replace(/\//g, "\\");
  const lower = norm.toLowerCase();
  const marker = "\\vulkansdk\\";
  const idx = lower.lastIndexOf(marker);
  if (idx === -1) return null;
  const tail = norm.slice(idx + marker.length);
  const parts = tail.split("\\");
  if (parts.length < 3) return null;
  if (!/^bin$/i.test(parts[1])) return null;
  const versionDir = parts[0];
  const base = norm.slice(0, idx);
  const sdkPath = path.join(base, "VulkanSDK", versionDir);
  return windowsVulkanSdkLayoutOk(sdkPath) ? sdkPath : null;
}

function discoverWindowsVulkanSdkFromVulkaninfoPath() {
  const whereExe = process.env.SystemRoot
    ? `${process.env.SystemRoot}\\System32\\where.exe`
    : "where.exe";
  const w = run(whereExe, ["vulkaninfo"]);
  if (!w.ok) return null;
  const lines = w.stdout
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => /\.exe$/i.test(l));
  for (const exe of lines) {
    const root = sdkRootFromVulkaninfoExe(exe);
    if (root) return root;
  }
  return null;
}

function discoverWindowsVulkanSdkFromRegistry() {
  const regExe = process.env.SystemRoot
    ? `${process.env.SystemRoot}\\System32\\reg.exe`
    : "reg.exe";
  const keys = [
    "HKLM\\SOFTWARE\\Khronos\\VulkanSDK",
    "HKLM\\SOFTWARE\\WOW6432Node\\Khronos\\VulkanSDK",
  ];
  let stdout = "";
  for (const key of keys) {
    const r = run(regExe, ["query", key, "/s", "/reg:64"]);
    if (r.ok && r.stdout) {
      stdout += r.stdout;
    }
  }
  if (!stdout) {
    return null;
  }
  const roots = [];
  for (const line of stdout.split(/\r?\n/)) {
    const m = line.match(/InstallDir\s+REG(?:_SZ|_EXPAND_SZ)\s+(.+?)\s*$/i);
    if (!m) continue;
    const candidate = m[1].trim();
    const normalized = normalizeWindowsVulkanSdkRoot(candidate);
    if (normalized) {
      roots.push(normalized);
    }
  }
  if (roots.length === 0) {
    return null;
  }
  const uniq = [...new Set(roots.map((p) => path.resolve(p)))];
  uniq.sort((a, b) => compareVersionNamesDesc(path.basename(a), path.basename(b)));
  return uniq[0] ?? null;
}

export function resolveWindowsVulkanSdkRoot() {
  const fromEnv = process.env.VULKAN_SDK;
  const normalizedEnv = normalizeWindowsVulkanSdkRoot(fromEnv);
  if (normalizedEnv) {
    return normalizedEnv;
  }
  if (hasPath(fromEnv) && windowsVulkanSdkLayoutOk(fromEnv)) {
    return path.resolve(fromEnv.trim());
  }
  return (
    discoverWindowsVulkanSdk() ??
    discoverWindowsVulkanSdkFromRegistry() ??
    discoverWindowsVulkanSdkFromVulkaninfoPath()
  );
}

export function vulkanSdkPresent() {
  return !!resolveWindowsVulkanSdkRoot();
}

export function applyDiscoveredVulkanSdkToProcessEnv() {
  const resolved = resolveWindowsVulkanSdkRoot();
  if (resolved) {
    process.env.VULKAN_SDK = resolved;
    return resolved;
  }
  return null;
}

function detectVulkanToolchain() {
  if (process.platform === "win32") {
    const envSnap = process.env.VULKAN_SDK;
    const resolved = resolveWindowsVulkanSdkRoot();
    if (resolved) {
      const source =
        typeof envSnap === "string" &&
        envSnap.trim() &&
        windowsVulkanSdkLayoutOk(envSnap) &&
        path.resolve(envSnap) === path.resolve(resolved)
          ? "env"
          : "discovered";
      return { ok: true, sdkPath: resolved, source };
    }
    return {
      ok: false,
      sdkPath: null,
      source: "missing",
      reason:
        "Vulkan SDK (Include + Lib) not found; set VULKAN_SDK to the SDK root or install Khronos Vulkan SDK",
    };
  }

  const fromEnv = process.env.VULKAN_SDK;
  if (hasPath(fromEnv)) {
    return { ok: true, sdkPath: fromEnv, source: "env" };
  }

  if (commandExists("vulkaninfo", ["--summary"])) {
    return { ok: true, sdkPath: null, source: "vulkaninfo" };
  }

  return {
    ok: false,
    sdkPath: null,
    source: "missing",
    reason: "Vulkan toolchain/runtime not detected",
  };
}

function autoSelectBackend() {
  if (process.platform === "darwin") {
    return {
      backend: "metal",
      reason: "macOS host uses Metal backend",
      gpuVendor: detectGpuVendor(),
      vulkanSdkPath: null,
    };
  }

  const gpuVendor = detectGpuVendor();
  const vulkan = detectVulkanToolchain();

  const hasNvidia = gpuVendor === "nvidia";
  if (hasNvidia && hasCudaToolchain()) {
    return {
      backend: "cuda",
      reason: "NVIDIA GPU + CUDA toolkit detected",
      gpuVendor,
      vulkanSdkPath: null,
    };
  }

  if (vulkan.ok) {
    const vkSource =
      vulkan.source === "env"
        ? "VULKAN_SDK"
        : vulkan.source === "discovered"
          ? "auto-discovered Vulkan SDK"
          : "vulkaninfo";
    return {
      backend: "vulkan",
      reason: hasNvidia
        ? `NVIDIA detected but CUDA toolkit missing; falling back to Vulkan (${vkSource})`
        : `Non-NVIDIA or unknown GPU; using Vulkan backend (${vkSource})`,
      gpuVendor,
      vulkanSdkPath: vulkan.sdkPath,
    };
  }

  return {
    backend: "none",
    reason: hasNvidia
      ? `NVIDIA detected but no buildable backend found (${vulkan.reason ?? "missing Vulkan"})`
      : `No GPU backend prerequisites detected (${vulkan.reason ?? "missing Vulkan"})`,
    gpuVendor,
    vulkanSdkPath: null,
  };
}

export function resolveBackend() {
  const forcedRaw = (process.env.WHISPER_GPU_BACKEND ?? "auto").trim().toLowerCase();
  if (forcedRaw !== "auto") {
    if (!VALID_BACKENDS.has(forcedRaw)) {
      throw new Error(
        `Invalid WHISPER_GPU_BACKEND="${forcedRaw}". Use auto|metal|cuda|vulkan|none.`,
      );
    }
    return {
      backend: forcedRaw,
      reason: "forced by WHISPER_GPU_BACKEND",
      forced: true,
      gpuVendor: detectGpuVendor(),
      vulkanSdkPath: detectVulkanToolchain().sdkPath,
    };
  }
  return { ...autoSelectBackend(), forced: false };
}

/**
 * Warn when SDKs are missing; no winget prompts (install manually — see README).
 * @param {Awaited<ReturnType<typeof resolveBackend>>} initial
 */
export function finalizeWindowsBackendSelection(initial) {
  if (process.platform !== "win32") {
    applyDiscoveredVulkanSdkToProcessEnv();
    return initial;
  }

  applyDiscoveredVulkanSdkToProcessEnv();
  let selected = initial;

  if (selected.gpuVendor === "nvidia" && !hasCudaToolchain()) {
    console.warn(
      "whisper-gpu: NVIDIA GPU detected but CUDA Toolkit not found. Install CUDA or set CUDA_PATH. See jarvis/README.md.",
    );
  }

  if (selected.backend === "vulkan" && !vulkanSdkPresent()) {
    if (selected.forced) {
      console.error(
        "whisper-gpu: WHISPER_GPU_BACKEND=vulkan but Vulkan SDK not found. Install SDK or set VULKAN_SDK.",
      );
      process.exit(1);
    }
    console.warn(
      "whisper-gpu: Vulkan SDK missing; switching to CPU-only Whisper for this run.",
    );
    return {
      backend: "none",
      reason: "Vulkan SDK not found (install Khronos Vulkan SDK or set VULKAN_SDK)",
      forced: false,
      gpuVendor: selected.gpuVendor,
      vulkanSdkPath: null,
    };
  }

  if (selected.backend === "cuda" && !hasCudaToolchain()) {
    if (selected.forced) {
      console.error(
        "whisper-gpu: WHISPER_GPU_BACKEND=cuda but CUDA toolkit not found. Install toolkit or set CUDA_PATH.",
      );
      process.exit(1);
    }
    const vulkan = detectVulkanToolchain();
    if (vulkan.ok) {
      console.warn(
        "whisper-gpu: CUDA toolkit missing; falling back to Vulkan for this run.",
      );
      return {
        backend: "vulkan",
        reason: "CUDA toolkit not installed; using Vulkan",
        forced: false,
        gpuVendor: selected.gpuVendor,
        vulkanSdkPath: vulkan.sdkPath,
      };
    }
    console.warn(
      "whisper-gpu: CUDA toolkit missing; switching to CPU-only Whisper for this run.",
    );
    return {
      backend: "none",
      reason: "CUDA toolkit not installed",
      forced: false,
      gpuVendor: selected.gpuVendor,
      vulkanSdkPath: null,
    };
  }

  if (selected.forced && selected.backend === "vulkan" && !vulkanSdkPresent()) {
    console.error(
      "whisper-gpu: WHISPER_GPU_BACKEND=vulkan but Vulkan SDK not found. Install SDK or set VULKAN_SDK.",
    );
    process.exit(1);
  }
  if (selected.forced && selected.backend === "cuda" && !hasCudaToolchain()) {
    console.error(
      "whisper-gpu: WHISPER_GPU_BACKEND=cuda but CUDA toolkit not found. Install toolkit or set CUDA_PATH.",
    );
    process.exit(1);
  }

  return selected;
}
