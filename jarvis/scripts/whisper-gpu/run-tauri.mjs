#!/usr/bin/env node

import fs from "fs";
import path from "path";
import process from "process";
import { spawn, spawnSync } from "child_process";
import { fileURLToPath } from "url";

import {
  finalizeWindowsBackendSelection,
  resolveBackend,
  resolveWindowsVulkanSdkRoot,
} from "./detect.mjs";
import {
  buildWindowsIsExecutableRunningScript,
  buildWindowsTerminateByExecutablePathScript,
  shouldReleaseWindowsJarvisExeLockForSubcommand,
} from "./launch.mjs";
import {
  formatResolvedCudaLog,
  prepareGpuNativeBuild,
  resolveBuildEnvironment,
} from "../build-environment.mjs";
import { collectBuildEnvDiagnostics } from "../diagnose-build-env.mjs";
import {
  formatCondensedBuildEnvDiagnostics,
  formatGpuBuildProgressBar,
  summarizeGpuNativeBuildProgress,
  tickGpuBuildProgressPoll,
} from "../gpu-build-progress.mjs";
import {
  checkCargoBuildLock,
  logFirstCudaBuildNotice,
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

/** Fetch wake + LLM GGUF into src-tauri/resources before Cargo needs bundle paths. */
function ensureBundledModels() {
  const script = path.join(JARVIS_ROOT, "scripts", "fetch-models.mjs");
  console.log("whisper-gpu: ensuring bundled models (skip if already on disk)...");
  const result = spawnSync(process.execPath, [script], {
    cwd: JARVIS_ROOT,
    stdio: "inherit",
    env: process.env,
  });
  if (result.status !== 0) {
    console.error("whisper-gpu: fetch-models failed");
    process.exit(result.status ?? 1);
  }
}

/**
 * @param {string[]} argv
 * @returns {{ extraArgs: string[], backendOverride: string | null }}
 */
function parseLauncherFlags(argv) {
  /** @type {string[]} */
  const extraArgs = [];
  let backendOverride = null;

  for (const arg of argv) {
    if (arg === "--cpu") {
      backendOverride = "none";
      continue;
    }
    if (arg === "--gpu") {
      backendOverride = "auto";
      continue;
    }
    extraArgs.push(arg);
  }

  return { extraArgs, backendOverride };
}

function releaseWindowsDevJarvisExeLock(subcommand) {
  if (
    process.platform !== "win32" ||
    !shouldReleaseWindowsJarvisExeLockForSubcommand(subcommand)
  ) {
    return;
  }
  for (const profile of ["debug", "release"]) {
    const exePath = path.join(JARVIS_ROOT, "src-tauri", "target", profile, "jarvis.exe");
    const script = buildWindowsTerminateByExecutablePathScript(exePath);
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
        `whisper-gpu: terminated ${killed} stale jarvis.exe process(es) (${profile}) to avoid Windows file-lock rebuild failure.`,
      );
    }
  }
}

function resolveJarvisDevExecutablePath(jarvisRoot, useReleaseNative) {
  const name = process.platform === "win32" ? "jarvis.exe" : "jarvis";
  const profile = useReleaseNative ? "release" : "debug";
  return path.join(jarvisRoot, "src-tauri", "target", profile, name);
}

function isJarvisDevProcessRunning(jarvisRoot, useReleaseNative) {
  const exePath = resolveJarvisDevExecutablePath(jarvisRoot, useReleaseNative);
  if (!fs.existsSync(exePath)) {
    return false;
  }
  if (process.platform === "win32") {
    const script = buildWindowsIsExecutableRunningScript(exePath);
    const r = spawnSync(
      "powershell",
      ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script],
      { cwd: jarvisRoot, encoding: "utf8" },
    );
    return r.status === 0;
  }
  const r = spawnSync("pgrep", ["-f", exePath], { encoding: "utf8" });
  return r.status === 0;
}

const GPU_PROGRESS_POLL_MS = 5_000;
const GPU_PROGRESS_NON_TTY_MS = 10_000;
/**
 * @param {string} line
 * @param {boolean} isTty
 */
function writeGpuProgressLine(line, isTty) {
  if (isTty) {
    process.stderr.write(`\r\x1b[2K${line}`);
  } else {
    console.warn(line);
  }
}

/**
 * @param {import("../gpu-build-progress.mjs").GpuNativeBuildProgressSnapshot | null} snapshot
 * @returns {string}
 */
function formatDevAppRunningProgressLine(snapshot) {
  if (snapshot && snapshot.overallPercent >= 100) {
    return "whisper-gpu: GPU native build ready — jarvis dev app running";
  }
  if (snapshot) {
    return `${formatGpuBuildProgressBar(snapshot)} — jarvis dev app running`;
  }
  return "whisper-gpu: jarvis dev app running — build progress monitoring stopped";
}

/**
 * @param {object} opts
 * @param {boolean} opts.isTty
 * @param {number} opts.startedAt
 * @param {import("../gpu-build-progress.mjs").GpuNativeBuildProgressSnapshot | null} opts.prevSnapshot
 * @param {number} opts.stallCount
 * @param {"debug"|"release"} opts.nativeProfile
 * @param {boolean} opts.prebuildWarm
 * @returns {{
 *   tick: ReturnType<typeof tickGpuBuildProgressPoll>,
 *   prevSnapshot: import("../gpu-build-progress.mjs").GpuNativeBuildProgressSnapshot,
 *   stallCount: number,
 *   lastNonTtyWrite: number,
 * }}
 */
function emitGpuProgressTick(opts) {
  const { isTty, startedAt, prevSnapshot, stallCount, nativeProfile, prebuildWarm } = opts;
  const tick = tickGpuBuildProgressPoll({
    jarvisRoot: JARVIS_ROOT,
    profile: nativeProfile,
    startedAt,
    prebuildWarm,
    prevSnapshot,
    stallCount,
  });

  const now = Date.now();
  let lastNonTtyWrite = opts.lastNonTtyWrite ?? 0;
  if (isTty || now - lastNonTtyWrite >= GPU_PROGRESS_NON_TTY_MS) {
    writeGpuProgressLine(tick.barLine, isTty);
    lastNonTtyWrite = now;
  }

  if (tick.debugBlock) {
    if (isTty) process.stderr.write("\n");
    console.warn(tick.debugBlock);
  }

  return {
    tick,
    prevSnapshot: tick.snapshot,
    stallCount: tick.stallCount,
    lastNonTtyWrite,
  };
}

/**
 * @param {string} spawnExecutable
 * @param {string[]} spawnArgv
 * @param {NodeJS.ProcessEnv} childEnv
 * @param {{
 *   cudaFirstBuild: boolean,
 *   subcommand: string,
 *   useReleaseNative: boolean,
 *   nativeProfile: "debug" | "release",
 *   prebuildWarm: boolean,
 * }} opts
 * @returns {Promise<number>}
 */
function spawnTauriWithGpuProgress(spawnExecutable, spawnArgv, childEnv, opts) {
  const { cudaFirstBuild, subcommand, useReleaseNative, nativeProfile, prebuildWarm } = opts;
  const progressEnabled =
    cudaFirstBuild && process.env.JARVIS_GPU_BUILD_PROGRESS !== "0";
  const stopProgressOnDevApp =
    subcommand === "dev"
      ? () => isJarvisDevProcessRunning(JARVIS_ROOT, useReleaseNative)
      : null;

  return new Promise((resolve, reject) => {
    const child = spawn(spawnExecutable, spawnArgv, {
      stdio: "inherit",
      cwd: JARVIS_ROOT,
      env: childEnv,
      windowsHide: false,
    });

    const startedAt = Date.now();
    let progressPoll = null;
    let devReadyPoll = null;
    /** @type {import("../gpu-build-progress.mjs").GpuNativeBuildProgressSnapshot | null} */
    let prevSnapshot = null;
    let stallCount = 0;
    const isTty = process.stderr.isTTY === true;

    const clearBuildTimers = () => {
      if (progressPoll) {
        clearInterval(progressPoll);
        progressPoll = null;
      }
      if (devReadyPoll) {
        clearInterval(devReadyPoll);
        devReadyPoll = null;
      }
      if (isTty && progressEnabled) {
        process.stderr.write("\n");
      }
    };

    if (progressEnabled) {
      let lastNonTtyWrite = 0;
      const runPoll = () => {
        if (stopProgressOnDevApp?.()) {
          console.warn(formatDevAppRunningProgressLine(prevSnapshot));
          clearBuildTimers();
          return;
        }

        const emitted = emitGpuProgressTick({
          isTty,
          startedAt,
          prevSnapshot,
          stallCount,
          nativeProfile,
          prebuildWarm,
          lastNonTtyWrite,
        });
        prevSnapshot = emitted.prevSnapshot;
        stallCount = emitted.stallCount;
        lastNonTtyWrite = emitted.lastNonTtyWrite;
      };

      runPoll();
      progressPoll = setInterval(runPoll, GPU_PROGRESS_POLL_MS);
    }

    if (stopProgressOnDevApp) {
      devReadyPoll = setInterval(() => {
        if (stopProgressOnDevApp()) {
          console.warn(formatDevAppRunningProgressLine(prevSnapshot));
          clearBuildTimers();
        }
      }, 2_000);
    }

    child.on("error", (err) => {
      clearBuildTimers();
      reject(err);
    });

    child.on("close", (code, signal) => {
      clearBuildTimers();
      if (signal) {
        resolve(128);
        return;
      }
      resolve(code ?? 1);
    });
  });
}

/**
 * Plain spawn without GPU progress polling (heartbeat removed; opt-out path).
 * @param {string} spawnExecutable
 * @param {string[]} spawnArgv
 * @param {NodeJS.ProcessEnv} childEnv
 * @param {{ subcommand: string, useReleaseNative: boolean }} opts
 * @returns {Promise<number>}
 */
function spawnTauriPlain(spawnExecutable, spawnArgv, childEnv, opts) {
  const { subcommand, useReleaseNative } = opts;
  const stopOnDevApp =
    subcommand === "dev"
      ? () => isJarvisDevProcessRunning(JARVIS_ROOT, useReleaseNative)
      : null;

  return new Promise((resolve, reject) => {
    const child = spawn(spawnExecutable, spawnArgv, {
      stdio: "inherit",
      cwd: JARVIS_ROOT,
      env: childEnv,
      windowsHide: false,
    });

    let devReadyPoll = null;
    const clearTimers = () => {
      if (devReadyPoll) {
        clearInterval(devReadyPoll);
        devReadyPoll = null;
      }
    };

    if (stopOnDevApp) {
      devReadyPoll = setInterval(() => {
        if (stopOnDevApp()) clearTimers();
      }, 15_000);
    }

    child.on("error", (err) => {
      clearTimers();
      reject(err);
    });

    child.on("close", (code, signal) => {
      clearTimers();
      if (signal) {
        resolve(128);
        return;
      }
      resolve(code ?? 1);
    });
  });
}

/**
 * CUDA dev uses release native profile to avoid MSVC debug stack overrun (see docs/bugs).
 * @param {string} subcommand
 * @param {string} backend
 * @param {string[]} extraArgs
 */
function applyGpuDevProfile(subcommand, backend, extraArgs) {
  if (subcommand !== "dev" || backend !== "cuda") {
    return extraArgs;
  }
  if (process.env.JARVIS_GPU_DEV_DEBUG === "1") {
    console.warn(
      "whisper-gpu: JARVIS_GPU_DEV_DEBUG=1 — using debug native profile (may crash on Whisper CUDA load).",
    );
    return extraArgs;
  }
  if (extraArgs.includes("--release")) {
    return extraArgs;
  }
  console.log(
    "whisper-gpu: CUDA dev uses --release native profile (stable GPU startup; set JARVIS_GPU_DEV_DEBUG=1 to opt out).",
  );
  return ["--release", ...extraArgs];
}

async function runTauri(subcommand, extraArgs, withGpuSelection) {
  const lock = checkCargoBuildLock(JARVIS_ROOT);
  if (lock.blocked) {
    console.error(`whisper-gpu: ${lock.message}`);
    process.exit(1);
  }

  if (subcommand === "dev" || subcommand === "build") {
    ensureBundledModels();
  }

  releaseWindowsDevJarvisExeLock(subcommand);

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

  const tauriExtraArgs = applyGpuDevProfile(subcommand, selected.backend, extraArgs);

  const nativeProfile =
    subcommand === "build" || tauriExtraArgs.includes("--release") ? "release" : "debug";

  const args = [subcommand];
  const cargoFeatures = ["llm-local"];
  if (withGpuSelection && selected.backend !== "none") {
    cargoFeatures.push(`whisper-${selected.backend}`, `llm-${selected.backend}`);
  }
  args.push("--features", cargoFeatures.join(","));
  args.push(...tauriExtraArgs);

  if (withGpuSelection) {
    console.log(
      `whisper-gpu: gpu_vendor=${selected.gpuVendor}; backend=${selected.backend} (${selected.reason})${selected.forced ? " [override]" : ""}`,
    );
    if (selected.backend === "none") {
      console.warn("whisper-gpu: building CPU-only Whisper backend.");
    }
    if (selected.backend === "cuda") {
      logFirstCudaBuildNotice(JARVIS_ROOT, nativeProfile);
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

  const { env: childEnv, cudaBuildEnv, generator, gpuPrebuild } = resolveBuildEnvironment({
    jarvisRoot: JARVIS_ROOT,
    channel: "tauri",
    baseEnv: process.env,
    backend: withGpuSelection ? selected.backend : "none",
    needsCuda: withGpuSelection && selected.backend === "cuda",
    logPrefix: "whisper-gpu",
  });

  if (cudaBuildEnv) {
    const cudaLog = formatResolvedCudaLog(cudaBuildEnv, gpuPrebuild);
    if (cudaLog) console.log(`whisper-gpu: ${cudaLog}`);
  }
  if (process.platform === "win32" && childEnv.LIBCLANG_PATH) {
    console.log(`whisper-gpu: set LIBCLANG_PATH=${childEnv.LIBCLANG_PATH} (whisper-rs-sys bindgen)`);
  }
  if (process.platform === "win32" && childEnv.BINDGEN_EXTRA_CLANG_ARGS) {
    console.log("whisper-gpu: set BINDGEN_EXTRA_CLANG_ARGS for MSVC + Windows SDK includes");
  }
  if (
    withGpuSelection &&
    selected.backend === "vulkan" &&
    process.platform === "win32" &&
    childEnv.VULKAN_SDK
  ) {
    const prev = process.env.VULKAN_SDK;
    if (!hasPath(prev) || path.resolve(prev) !== path.resolve(childEnv.VULKAN_SDK)) {
      console.log(`whisper-gpu: set VULKAN_SDK=${childEnv.VULKAN_SDK}`);
    }
  }

  if (withGpuSelection && process.platform === "win32" && selected.backend === "cuda") {
    prepareGpuNativeBuild(JARVIS_ROOT, childEnv, {
      logPrefix: "whisper-gpu",
      profile: nativeProfile,
      generator: generator ?? undefined,
    });
  }

  const cudaGpuDev = withGpuSelection && selected.backend === "cuda";
  const prebuildWarm = childEnv.JARVIS_GPU_PREBUILD_WARM === "1" || gpuPrebuild?.warm === true;
  const gpuBuildSnapshot = cudaGpuDev
    ? summarizeGpuNativeBuildProgress(JARVIS_ROOT, nativeProfile, { prebuildWarm })
    : null;
  const gpuBuildNeedsProgress =
    cudaGpuDev &&
    process.env.JARVIS_GPU_BUILD_PROGRESS !== "0" &&
    (gpuBuildSnapshot?.overallPercent ?? 0) < 100;

  if (cudaGpuDev) {
    for (const line of formatCondensedBuildEnvDiagnostics(
      collectBuildEnvDiagnostics(JARVIS_ROOT, nativeProfile),
    )) {
      console.log(line);
    }
    if (!prebuildWarm) {
      console.warn(
        "whisper-gpu: GPU prebuild cache is COLD — expect 45–90+ min unless you run: npm run prebuild:gpu-cuda",
      );
    }
    if (process.env.JARVIS_GPU_BUILD_PROGRESS !== "0") {
      if (gpuBuildNeedsProgress) {
        console.log(
          "whisper-gpu: GPU build progress bar enabled (5s updates on stderr; set JARVIS_GPU_BUILD_PROGRESS=0 to disable)",
        );
      } else if (gpuBuildSnapshot) {
        console.log(
          `${formatGpuBuildProgressBar({ ...gpuBuildSnapshot, elapsedSec: 0 })} — CUDA cache warm (incremental cargo only)`,
        );
      }
    }
  }

  let status;
  try {
    const spawnOpts = {
      subcommand,
      useReleaseNative: tauriExtraArgs.includes("--release"),
    };
    if (gpuBuildNeedsProgress) {
      status = await spawnTauriWithGpuProgress(spawnExecutable, spawnArgv, childEnv, {
        ...spawnOpts,
        cudaFirstBuild: true,
        nativeProfile,
        prebuildWarm,
      });
    } else {
      status = await spawnTauriPlain(spawnExecutable, spawnArgv, childEnv, spawnOpts);
    }
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
  const cleanupDevChildren = () => {
    releaseWindowsDevJarvisExeLock("dev");
    releaseWindowsDevJarvisExeLock("build");
  };
  process.once("SIGINT", cleanupDevChildren);
  process.once("SIGTERM", cleanupDevChildren);
  if (process.platform === "win32") {
    process.once("SIGBREAK", cleanupDevChildren);
  }

  try {
    const rawArgv = process.argv.slice(2);
    const { extraArgs, backendOverride } = parseLauncherFlags(rawArgv);
    if (backendOverride === "none") {
      process.env.WHISPER_GPU_BACKEND = "none";
    } else if (backendOverride === "auto") {
      process.env.WHISPER_GPU_BACKEND = "auto";
    }
    const [subcommand = "build", ...rest] = extraArgs;
    await runTauri(subcommand, rest, ["build", "dev"].includes(subcommand));
  } catch (e) {
    console.error("whisper-gpu:", e instanceof Error ? e.message : e);
    process.exit(1);
  }
}

main();
