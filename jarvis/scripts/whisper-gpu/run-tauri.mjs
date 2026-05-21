#!/usr/bin/env node

import fs from "fs";
import path from "path";
import process from "process";
import { spawn, spawnSync } from "child_process";
import { fileURLToPath } from "url";

import {
  applyDiscoveredCudaToolkitToProcessEnv,
  applyWindowsCudaBuildEnvIfNeeded,
  finalizeWindowsBackendSelection,
  resolveBackend,
  resolveWindowsVulkanSdkRoot,
} from "./detect.mjs";
import {
  buildWindowsTerminateByExecutablePathScript,
  prependWindowsPathEntries,
  shouldReleaseWindowsJarvisExeLockForSubcommand,
} from "./launch.mjs";
import {
  assertWindowsWhisperBindgenEnv,
  buildWindowsWhisperCargoEnv,
} from "./win-env.mjs";
import {
  checkCargoBuildLock,
  logFirstCudaBuildNotice,
  warnIfCmakeGeneratorMismatch,
} from "./preflight.mjs";

const JARVIS_ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "..");

function hasPath(dir) {
  return typeof dir === "string" && dir.trim().length > 0 && fs.existsSync(dir);
}

function resolveTauriCli() {
  const p = path.join(JARVIS_ROOT, "node_modules", "@tauri-apps", "cli", "tauri.js");
  if (!fs.existsSync(p)) {
    throw new Error(
      `Tauri CLI not found at ${p}. Run npm install in the jarvis folder.`,
    );
  }
  return p;
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
      `whisper-gpu: terminated ${killed} stale jarvis.exe process(es) to avoid Windows file-lock rebuild failure.`,
    );
  }
}

function buildChildEnv(withGpuSelection, selected) {
  const childEnv = { ...process.env };
  childEnv.CARGO_TERM_PROGRESS = childEnv.CARGO_TERM_PROGRESS ?? "always";

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
        `whisper-gpu: configured CUDA build env (generator=${cudaBuildEnv.generator}; CUDA_PATH=${cudaBuildEnv.cudaRoot})`,
      );
    }
  } else if (process.platform === "win32" && !childEnv.CMAKE_GENERATOR?.trim()) {
    childEnv.CMAKE_GENERATOR = "Visual Studio 17 2022";
  }

  if (process.platform === "win32") {
    assertWindowsWhisperBindgenEnv("whisper-gpu");
    const whisperEnv = buildWindowsWhisperCargoEnv(childEnv, { force: true });
    Object.assign(childEnv, whisperEnv);
    if (whisperEnv.LIBCLANG_PATH) {
      console.log(`whisper-gpu: set LIBCLANG_PATH=${whisperEnv.LIBCLANG_PATH} (whisper-rs-sys bindgen)`);
    }
    if (whisperEnv.BINDGEN_EXTRA_CLANG_ARGS) {
      console.log("whisper-gpu: set BINDGEN_EXTRA_CLANG_ARGS for MSVC + Windows SDK includes");
    }
  }

  if (withGpuSelection && selected.backend === "vulkan" && process.platform === "win32") {
    const vkRoot = resolveWindowsVulkanSdkRoot();
    if (vkRoot) {
      childEnv.VULKAN_SDK = vkRoot;
      const prev = process.env.VULKAN_SDK;
      if (!hasPath(prev) || path.resolve(prev) !== path.resolve(vkRoot)) {
        console.log(`whisper-gpu: set VULKAN_SDK=${vkRoot}`);
      }
    }
  }

  return childEnv;
}

/**
 * @param {string} spawnExecutable
 * @param {string[]} spawnArgv
 * @param {NodeJS.ProcessEnv} childEnv
 * @param {boolean} cudaFirstBuild
 * @returns {Promise<number>}
 */
function spawnTauriWithHeartbeat(spawnExecutable, spawnArgv, childEnv, cudaFirstBuild) {
  return new Promise((resolve, reject) => {
    const child = spawn(spawnExecutable, spawnArgv, {
      stdio: "inherit",
      cwd: JARVIS_ROOT,
      env: childEnv,
      windowsHide: false,
    });

    const started = Date.now();
    const heartbeat = setInterval(() => {
      const mins = Math.floor((Date.now() - started) / 60_000);
      if (mins < 1) return;
      const hint = cudaFirstBuild
        ? " (first whisper-cuda build can take 30+ min)"
        : "";
      console.warn(`whisper-gpu: still building… ${mins} min elapsed${hint}`);
    }, 60_000);

    child.on("error", (err) => {
      clearInterval(heartbeat);
      reject(err);
    });

    child.on("close", (code, signal) => {
      clearInterval(heartbeat);
      if (signal) {
        resolve(128);
        return;
      }
      resolve(code ?? 1);
    });
  });
}

async function runTauri(subcommand, extraArgs, withGpuSelection) {
  const lock = checkCargoBuildLock(JARVIS_ROOT);
  if (lock.blocked) {
    console.error(`whisper-gpu: ${lock.message}`);
    process.exit(1);
  }

  releaseWindowsDevJarvisExeLock(subcommand);
  applyDiscoveredCudaToolkitToProcessEnv();

  let selected = resolveBackend();
  if (withGpuSelection) {
    selected = finalizeWindowsBackendSelection(selected);
  }

  if (
    withGpuSelection &&
    process.platform === "win32" &&
    selected.backend === "vulkan" &&
    !resolveWindowsVulkanSdkRoot()
  ) {
    if (selected.forced) {
      console.error(
        "whisper-gpu: whisper-vulkan needs Vulkan SDK root (Include + Lib). Install SDK or set VULKAN_SDK.",
      );
      process.exit(1);
    }
    console.warn(
      "whisper-gpu: Vulkan SDK not resolvable; switching to CPU-only Whisper for this run.",
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
      `whisper-gpu: gpu_vendor=${selected.gpuVendor}; backend=${selected.backend} (${selected.reason})${selected.forced ? " [override]" : ""}`,
    );
    if (selected.backend === "none") {
      console.warn("whisper-gpu: building CPU-only Whisper backend.");
    }
    if (selected.backend === "cuda") {
      logFirstCudaBuildNotice(JARVIS_ROOT);
    }
  } else {
    console.log(
      `whisper-gpu: passthrough mode for "${subcommand}" (no whisper feature auto-select)`,
    );
  }

  const tauriCli = resolveTauriCli();
  const spawnExecutable = process.execPath;
  const spawnArgv = [tauriCli, ...args];
  console.log(
    `whisper-gpu: exec node ${path.relative(JARVIS_ROOT, tauriCli)} ${args.join(" ")}`,
  );

  const childEnv = buildChildEnv(withGpuSelection, selected);
  if (withGpuSelection && process.platform === "win32") {
    const intended =
      selected.backend === "cuda"
        ? childEnv.CMAKE_GENERATOR ?? "NMake Makefiles"
        : childEnv.CMAKE_GENERATOR ?? "Visual Studio 17 2022";
    warnIfCmakeGeneratorMismatch(JARVIS_ROOT, intended);
  }
  const cudaFirstBuild =
    withGpuSelection && selected.backend === "cuda";

  let status;
  try {
    status = await spawnTauriWithHeartbeat(
      spawnExecutable,
      spawnArgv,
      childEnv,
      cudaFirstBuild,
    );
  } catch (err) {
    console.error(
      "whisper-gpu: failed to spawn Tauri CLI:",
      err instanceof Error ? err.message : err,
    );
    process.exit(1);
  }

  if (status !== 0 && withGpuSelection && selected.backend === "vulkan") {
    console.error(
      "whisper-gpu: build failed. For whisper-vulkan, ensure Vulkan SDK (VULKAN_SDK), CMake, and a C++ toolchain are installed; see jarvis/README.md.",
    );
  }
  process.exit(status);
}

async function main() {
  try {
    const [subcommand = "build", ...rest] = process.argv.slice(2);
    await runTauri(subcommand, rest, ["build", "dev"].includes(subcommand));
  } catch (e) {
    console.error("whisper-gpu:", e instanceof Error ? e.message : e);
    process.exit(1);
  }
}

main();
