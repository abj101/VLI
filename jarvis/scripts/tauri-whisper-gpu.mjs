#!/usr/bin/env node

import fs from "fs";
import path from "path";
import process from "process";
import readline from "readline";
import { spawnSync } from "child_process";
import { fileURLToPath } from "url";

import {
  buildWindowsTerminateByExecutablePathScript,
  buildWingetInstallArgs,
  isWingetInstallSuccessStatus,
  prependWindowsPathEntries,
  shouldReleaseWindowsJarvisExeLockForSubcommand,
} from "./tauri-whisper-gpu-launch.mjs";
import {
  normalizeWindowsVulkanSdkRoot,
  windowsVulkanSdkLayoutOk,
} from "./tauri-whisper-gpu-win-sdk.mjs";

const VALID_BACKENDS = new Set(["metal", "cuda", "vulkan", "none"]);

const JARVIS_ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");

const WINGET_VULKAN_ID = "KhronosGroup.VulkanSDK";
const WINGET_CUDA_ID = "Nvidia.CUDA";

function resolveTauriCli() {
  const p = path.join(JARVIS_ROOT, "node_modules", "@tauri-apps", "cli", "tauri.js");
  if (!fs.existsSync(p)) {
    throw new Error(
      `Tauri CLI not found at ${p}. Run npm install in the jarvis folder.`,
    );
  }
  return p;
}

function run(cmd, args) {
  const out = spawnSync(cmd, args, { encoding: "utf8" });
  return {
    ok: out.status === 0,
    stdout: out.stdout ?? "",
    stderr: out.stderr ?? "",
  };
}

function commandExists(cmd, args = ["--version"]) {
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

function detectGpuVendor() {
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

function resolveWindowsCudaToolkitRoot() {
  const fromEnv = process.env.CUDA_PATH;
  if (hasPath(fromEnv) && windowsCudaToolkitLayoutOk(fromEnv)) {
    return path.resolve(fromEnv.trim());
  }
  return discoverWindowsCudaToolkitRoot();
}

function applyDiscoveredCudaToolkitToProcessEnv() {
  if (process.platform !== "win32") return null;
  const resolved = resolveWindowsCudaToolkitRoot();
  if (resolved) {
    process.env.CUDA_PATH = resolved;
    return resolved;
  }
  return null;
}

function hasCudaToolchain() {
  if (process.platform === "win32") {
    return !!resolveWindowsCudaToolkitRoot() || commandExists("nvcc");
  }
  const cudaPath = process.env.CUDA_PATH;
  if (hasPath(cudaPath)) return true;
  return commandExists("nvcc");
}

function parseVersionParts(name) {
  return name.split(".").map((p) => Number.parseInt(p, 10) || 0);
}

function compareVersionNamesDesc(a, b) {
  const av = parseVersionParts(a);
  const bv = parseVersionParts(b);
  const n = Math.max(av.length, bv.length);
  for (let i = 0; i < n; i += 1) {
    const x = av[i] ?? 0;
    const y = bv[i] ?? 0;
    if (x !== y) return y - x;
  }
  return 0;
}

function discoverWindowsMsvcHostX64BinDir() {
  const roots = [
    "C:\\Program Files (x86)\\Microsoft Visual Studio\\2022\\BuildTools\\VC\\Tools\\MSVC",
    "C:\\Program Files\\Microsoft Visual Studio\\2022\\Community\\VC\\Tools\\MSVC",
    "C:\\Program Files\\Microsoft Visual Studio\\2022\\Professional\\VC\\Tools\\MSVC",
    "C:\\Program Files\\Microsoft Visual Studio\\2022\\Enterprise\\VC\\Tools\\MSVC",
  ];
  for (const root of roots) {
    if (!fs.existsSync(root)) continue;
    const versions = fs
      .readdirSync(root, { withFileTypes: true })
      .filter((d) => d.isDirectory())
      .map((d) => d.name)
      .sort(compareVersionNamesDesc);
    for (const version of versions) {
      const hostBin = path.join(root, version, "bin", "Hostx64", "x64");
      if (fs.existsSync(path.join(hostBin, "cl.exe"))) {
        return hostBin;
      }
    }
  }
  return null;
}

function applyWindowsCudaBuildEnvIfNeeded(envObj) {
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
  // NMake generator rejects Visual Studio-specific instance/platform/toolset hints.
  // Parent shells (IDE/VS Developer prompts) may export these and break CMake configure.
  if (envObj.CMAKE_GENERATOR === "NMake Makefiles") {
    delete envObj.CMAKE_GENERATOR_INSTANCE;
    delete envObj.CMAKE_GENERATOR_PLATFORM;
    delete envObj.CMAKE_GENERATOR_TOOLSET;
  }
  if (!envObj.CMAKE_MAKE_PROGRAM) envObj.CMAKE_MAKE_PROGRAM = nmakeExe;
  if (!envObj.CMAKE_CUDA_COMPILER) envObj.CMAKE_CUDA_COMPILER = nvccExe;
  if (!envObj.CMAKE_CUDA_HOST_COMPILER) envObj.CMAKE_CUDA_HOST_COMPILER = clExe;
  if (!envObj.CMAKE_CUDA_FLAGS) envObj.CMAKE_CUDA_FLAGS = "-Xcompiler=/Zc:preprocessor";
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

/**
 * `vulkaninfo` ships with the Vulkan SDK under `<sdk>\\Bin\\vulkaninfo.exe`, and also as a small
 * Windows component under `System32` (runtime only — not a compilable SDK). Only trust paths that
 * sit under a `VulkanSDK` install tree.
 */
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

/**
 * Vulkan SDK Windows installer registers `InstallDir` under HKLM\\SOFTWARE\\Khronos\\VulkanSDK.
 */
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

/**
 * Prefer a layout-valid SDK root: env wins if good; else folder scan; else vulkaninfo path hint.
 */
function resolveWindowsVulkanSdkRoot() {
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

function vulkanSdkPresent() {
  return !!resolveWindowsVulkanSdkRoot();
}

function applyDiscoveredVulkanSdkToProcessEnv() {
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
  const gpuVendor = detectGpuVendor();
  const vulkan = detectVulkanToolchain();
  if (process.platform === "darwin") {
    return {
      backend: "metal",
      reason: "macOS host uses Metal backend",
      gpuVendor,
      vulkanSdkPath: null,
    };
  }

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

function resolveBackend() {
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

function wingetAvailable() {
  return run("winget", ["--version"]).ok;
}

let wingetDisableInteractivitySupported;

function wingetInstallSupportsDisableInteractivity() {
  if (wingetDisableInteractivitySupported !== undefined) {
    return wingetDisableInteractivitySupported;
  }
  const h = run("winget", ["install", "--help"]);
  wingetDisableInteractivitySupported =
    h.ok && /--disable-interactivity\b/i.test(`${h.stdout}\n${h.stderr}`);
  return wingetDisableInteractivitySupported;
}

function runWingetInstall(packageId, label) {
  console.log(
    `tauri-whisper-gpu: installing ${label} via winget (this may take several minutes)…`,
  );
  const r = spawnSync(
    "winget",
    buildWingetInstallArgs(packageId, {
      disableInteractivity: wingetInstallSupportsDisableInteractivity(),
    }),
    {
      stdio: "inherit",
      cwd: JARVIS_ROOT,
      env: process.env,
    },
  );
  const status = r.status ?? 1;
  const ok = isWingetInstallSuccessStatus(status);
  if (!ok) {
    console.error(
      `tauri-whisper-gpu: winget install failed for ${packageId} (exit ${status}).`,
    );
    console.error(
      `tauri-whisper-gpu: try elevated PowerShell: winget install -e --id ${packageId}  (Store / policy / admin). CUDA installer: https://developer.nvidia.com/cuda-downloads`,
    );
  } else if ((status >>> 0) === 0x8a15002b) {
    console.log(
      `tauri-whisper-gpu: ${packageId} already up to date (winget update not applicable).`,
    );
  }
  return ok;
}

function canPromptInteractively() {
  return Boolean(process.stdin.isTTY && process.stdout.isTTY);
}

function releaseWindowsDevJarvisExeLock(subcommand) {
  if (
    process.platform !== "win32" ||
    !shouldReleaseWindowsJarvisExeLockForSubcommand(subcommand)
  ) {
    return;
  }
  const debugExePath = path.join(JARVIS_ROOT, "src-tauri", "target", "debug", "jarvis.exe");
  const script = buildWindowsTerminateByExecutablePathScript(debugExePath);
  const r = spawnSync(
    "powershell",
    ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script],
    {
      cwd: JARVIS_ROOT,
      env: process.env,
      encoding: "utf8",
    },
  );
  const killed = Number.parseInt((r.stdout ?? "").trim(), 10);
  if (Number.isInteger(killed) && killed > 0) {
    console.warn(
      `tauri-whisper-gpu: terminated ${killed} stale jarvis.exe process(es) to avoid Windows file-lock rebuild failure.`,
    );
  }
}

function promptYesNo(question, defaultNo = true) {
  if (!canPromptInteractively()) {
    console.warn(
      "tauri-whisper-gpu: non-interactive terminal; skipping install prompt. Set VULKAN_SDK / CUDA_PATH or run winget manually.",
    );
    return Promise.resolve(false);
  }
  const rl = readline.createInterface({
    input: process.stdin,
    output: process.stdout,
  });
  return new Promise((resolve) => {
    rl.question(question, (answer) => {
      rl.close();
      const a = answer.trim().toLowerCase();
      if (!a) {
        resolve(!defaultNo);
        return;
      }
      resolve(a === "y" || a === "yes");
    });
  });
}

async function ensureWindowsWhisperGpuPrereqs(initial) {
  if (process.platform !== "win32") return initial;
  const forcedBackend = initial.forced ? initial.backend : null;
  let selected = initial;

  if (process.env.WHISPER_SKIP_PREREQ_PROMPT === "1") {
    console.log("tauri-whisper-gpu: WHISPER_SKIP_PREREQ_PROMPT=1; skipping install prompts.");
    applyDiscoveredVulkanSdkToProcessEnv();
    return selected;
  }
  if (!wingetAvailable()) {
    console.warn(
      "tauri-whisper-gpu: winget not available; cannot auto-install Vulkan/CUDA. Install SDKs manually.",
    );
    applyDiscoveredVulkanSdkToProcessEnv();
    return selected;
  }

  let didInstallAttempt = false;
  let nvidiaCudaPromptDone = false;

  if (selected.gpuVendor === "nvidia" && !hasCudaToolchain()) {
    nvidiaCudaPromptDone = true;
    const ok = await promptYesNo(
      "NVIDIA GPU detected, but CUDA Toolkit is not installed (large download). Install Nvidia.CUDA via winget now? [y/N] ",
      true,
    );
    if (ok) {
      didInstallAttempt = true;
      runWingetInstall(WINGET_CUDA_ID, "NVIDIA CUDA Toolkit");
      selected = resolveBackend();
    }
  }

  if (selected.backend === "vulkan" && !vulkanSdkPresent()) {
    const ok = await promptYesNo(
      `Whisper Vulkan needs Vulkan SDK (VULKAN_SDK). Install ${WINGET_VULKAN_ID} with winget now? [y/N] `,
      true,
    );
    if (ok) {
      didInstallAttempt = true;
      runWingetInstall(WINGET_VULKAN_ID, "Vulkan SDK");
    } else if (selected.forced) {
      console.error("tauri-whisper-gpu: Vulkan SDK required for WHISPER_GPU_BACKEND=vulkan.");
      process.exit(1);
    } else {
      console.warn(
        "tauri-whisper-gpu: Vulkan SDK missing; falling back to CPU-only Whisper for this run.",
      );
      applyDiscoveredVulkanSdkToProcessEnv();
      return {
        backend: "none",
        reason: "Vulkan SDK not installed (declined winget prompt)",
        forced: false,
        gpuVendor: selected.gpuVendor,
        vulkanSdkPath: null,
      };
    }
  }

  if (selected.backend === "cuda" && !hasCudaToolchain() && !nvidiaCudaPromptDone) {
    const ok = await promptYesNo(
      `Whisper CUDA needs NVIDIA CUDA Toolkit. Install ${WINGET_CUDA_ID} with winget now? [y/N] `,
      true,
    );
    if (ok) {
      didInstallAttempt = true;
      runWingetInstall(WINGET_CUDA_ID, "NVIDIA CUDA Toolkit");
    } else if (selected.forced) {
      console.error("tauri-whisper-gpu: CUDA toolkit required for WHISPER_GPU_BACKEND=cuda.");
      process.exit(1);
    } else {
      const vulkan = detectVulkanToolchain();
      if (vulkan.ok) {
        const vkSource =
          vulkan.source === "env"
            ? "VULKAN_SDK"
            : vulkan.source === "discovered"
              ? "auto-discovered Vulkan SDK"
              : "vulkaninfo";
        console.warn(
          "tauri-whisper-gpu: CUDA toolkit missing; falling back to Vulkan for this run.",
        );
        applyDiscoveredVulkanSdkToProcessEnv();
        return {
          backend: "vulkan",
          reason: `CUDA toolkit not installed (declined winget prompt); using Vulkan (${vkSource})`,
          forced: false,
          gpuVendor: selected.gpuVendor,
          vulkanSdkPath: vulkan.sdkPath,
        };
      }
      console.warn(
        "tauri-whisper-gpu: CUDA toolkit missing; falling back to CPU-only Whisper for this run.",
      );
      return {
        backend: "none",
        reason: "CUDA toolkit not installed (declined winget prompt)",
        forced: false,
        gpuVendor: selected.gpuVendor,
        vulkanSdkPath: null,
      };
    }
  }

  if (selected.backend === "none" && !selected.forced) {
    const ok = await promptYesNo(
      `No GPU Whisper backend detected. Install Vulkan SDK (${WINGET_VULKAN_ID}) for Vulkan builds? [y/N] `,
      true,
    );
    if (ok) {
      didInstallAttempt = true;
      runWingetInstall(WINGET_VULKAN_ID, "Vulkan SDK");
    }
  }

  applyDiscoveredVulkanSdkToProcessEnv();

  if (didInstallAttempt) {
    const next = resolveBackend();
    if (forcedBackend === "vulkan" && !vulkanSdkPresent()) {
      console.error(
        "tauri-whisper-gpu: Vulkan SDK still missing. Set VULKAN_SDK or re-open terminal after SDK install.",
      );
      process.exit(1);
    }
    if (forcedBackend === "cuda" && !hasCudaToolchain()) {
      console.error(
        "tauri-whisper-gpu: CUDA toolkit still missing. Set CUDA_PATH / PATH for nvcc, or open a new terminal after install.",
      );
      process.exit(1);
    }
    return next;
  }

  if (forcedBackend === "vulkan" && !vulkanSdkPresent()) {
    console.error(
      "tauri-whisper-gpu: WHISPER_GPU_BACKEND=vulkan but Vulkan SDK not found. Install SDK or set VULKAN_SDK.",
    );
    process.exit(1);
  }
  if (forcedBackend === "cuda" && !hasCudaToolchain()) {
    console.error(
      "tauri-whisper-gpu: WHISPER_GPU_BACKEND=cuda but CUDA toolkit not found. Install toolkit or set CUDA_PATH.",
    );
    process.exit(1);
  }

  return selected;
}

async function runTauri(subcommand, extraArgs, withGpuSelection) {
  releaseWindowsDevJarvisExeLock(subcommand);
  applyDiscoveredCudaToolkitToProcessEnv();

  let selected = resolveBackend();
  if (withGpuSelection && process.platform === "win32") {
    selected = await ensureWindowsWhisperGpuPrereqs(selected);
  } else if (withGpuSelection) {
    applyDiscoveredVulkanSdkToProcessEnv();
  }

  if (
    withGpuSelection &&
    process.platform === "win32" &&
    selected.backend === "vulkan" &&
    !resolveWindowsVulkanSdkRoot()
  ) {
    if (selected.forced) {
      console.error(
        "tauri-whisper-gpu: whisper-vulkan needs Vulkan SDK root (Include + Lib). Install SDK or set VULKAN_SDK.",
      );
      process.exit(1);
    }
    console.warn(
      "tauri-whisper-gpu: Vulkan SDK not resolvable; switching to CPU-only Whisper for this run.",
    );
    selected = {
      backend: "none",
      reason: "Vulkan SDK not found (Cargo needs VULKAN_SDK on Windows for whisper-rs-sys)",
      forced: false,
      gpuVendor: selected.gpuVendor,
      vulkanSdkPath: null,
    };
  }

  const args = [subcommand];

  if (withGpuSelection && selected.backend !== "none") {
    args.push("--features", `whisper-${selected.backend}`);
  }
  args.push(...extraArgs);

  if (withGpuSelection) {
    console.log(
      `tauri-whisper-gpu: gpu_vendor=${selected.gpuVendor}; backend=${selected.backend} (${selected.reason})${selected.forced ? " [override]" : ""}`,
    );
    if (selected.backend === "none") {
      console.warn("tauri-whisper-gpu: building CPU-only Whisper backend.");
    }
  } else {
    console.log(
      `tauri-whisper-gpu: passthrough mode for "${subcommand}" (no whisper feature auto-select)`,
    );
  }

  const tauriCli = resolveTauriCli();
  const spawnExecutable = process.execPath;
  const spawnArgv = [tauriCli, ...args];
  console.log(
    `tauri-whisper-gpu: exec node ${path.relative(JARVIS_ROOT, tauriCli)} ${args.join(" ")}`,
  );

  const childEnv = { ...process.env };
  if (withGpuSelection && selected.backend === "cuda" && process.platform === "win32") {
    const cudaBuildEnv = applyWindowsCudaBuildEnvIfNeeded(childEnv);
    if (cudaBuildEnv) {
      const cudaBin = path.join(cudaBuildEnv.cudaRoot, "bin");
      const cudaBinX64 = path.join(cudaBuildEnv.cudaRoot, "bin", "x64");
      const nvtxBin = path.join(cudaBuildEnv.cudaRoot, "extras", "CUPTI", "lib64");
      const existingPath =
        childEnv.PATH ?? childEnv.Path ?? process.env.PATH ?? process.env.Path ?? "";
      const mergedPath = prependWindowsPathEntries(existingPath, [cudaBinX64, cudaBin, nvtxBin]);
      childEnv.PATH = mergedPath;
      childEnv.Path = mergedPath;
      console.log(
        `tauri-whisper-gpu: configured CUDA build env (generator=${cudaBuildEnv.generator}; CUDA_PATH=${cudaBuildEnv.cudaRoot})`,
      );
    }
  }
  if (withGpuSelection && selected.backend === "vulkan" && process.platform === "win32") {
    const vkRoot = resolveWindowsVulkanSdkRoot();
    if (vkRoot) {
      childEnv.VULKAN_SDK = vkRoot;
      const prev = process.env.VULKAN_SDK;
      if (!hasPath(prev) || path.resolve(prev) !== path.resolve(vkRoot)) {
        console.log(`tauri-whisper-gpu: set VULKAN_SDK=${vkRoot}`);
      }
    }
  }

  const child = spawnSync(spawnExecutable, spawnArgv, {
    stdio: "inherit",
    cwd: JARVIS_ROOT,
    env: childEnv,
    windowsVerbatimArguments: false,
  });
  if (child.error) {
    console.error("tauri-whisper-gpu: failed to spawn Tauri CLI:", child.error.message);
    process.exit(1);
  }
  if ((child.status ?? 1) !== 0 && withGpuSelection && selected.backend === "vulkan") {
    console.error(
      "tauri-whisper-gpu: build failed. For whisper-vulkan, ensure Vulkan SDK (VULKAN_SDK), CMake, and a C++ toolchain are installed; see jarvis/README.md.",
    );
  }
  process.exit(child.status ?? 1);
}

async function main() {
  try {
    const [subcommand = "build", ...rest] = process.argv.slice(2);
    await runTauri(subcommand, rest, ["build", "dev"].includes(subcommand));
  } catch (e) {
    console.error("tauri-whisper-gpu:", e instanceof Error ? e.message : e);
    process.exit(1);
  }
}

main().catch((e) => {
  console.error("tauri-whisper-gpu:", e instanceof Error ? e.message : e);
  process.exit(1);
});
