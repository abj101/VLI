/**
 * Unified Build Environment seam — one interface for tauri, cargo, and sync channels.
 */

import path from "path";

import {
  applyDiscoveredCudaToolkitToProcessEnv,
  applyWindowsCudaBuildEnvIfNeeded,
  formatCudaBuildProfileLog,
  resolveWindowsVulkanSdkRoot,
} from "./whisper-gpu/detect.mjs";
import { prependWindowsPathEntries } from "./whisper-gpu/launch.mjs";
import {
  assertWindowsWhisperBindgenEnv,
  buildWindowsWhisperCargoEnv,
} from "./whisper-gpu/win-env.mjs";
import { applyGpuPrebuildEnv, purgeWindowsStaleCudaCompilerLauncherCaches, seedGpuPrebuildCache } from "./gpu-prebuild.mjs";
import { clearUnusableGpuSysCmakeCaches } from "./gpu-native-sys.mjs";
import { warnIfCmakeGeneratorMismatch } from "./whisper-gpu/preflight.mjs";

/**
 * Entry point that resolves the build environment.
 *
 * - `tauri` — `run-tauri.mjs` (full CUDA + bindgen + prebuild env)
 * - `cargo` — `cargo-win-env.mjs` (full path when CUDA features requested)
 * - `sync` — `sync-cargo-win-env.mjs` (discovery only; writes `config.local.toml`)
 *
 * `sync` callers should pass `needsCuda: true` on Windows even for CPU-only workflows so
 * `discovered.CMAKE_GENERATOR` matches the Ninja/NMake generator used by the GPU path,
 * avoiding CMake cache mismatches when switching CPU ↔ CUDA features.
 *
 * @typedef {"tauri" | "cargo" | "sync" | "prebuild"} BuildEnvChannel
 */

/**
 * Resolve a frozen build-environment snapshot for the given channel.
 *
 * **Invariant:** Callers must pass the returned `env` to spawned children (`cargo`, `tauri`, etc.).
 * Do not rely on `process.env` after this call — non-`sync` CUDA paths may mutate global
 * `process.env` via toolkit discovery (`applyDiscoveredCudaToolkitToProcessEnv`).
 *
 * @param {object} opts
 * @param {string} opts.jarvisRoot
 * @param {BuildEnvChannel} opts.channel
 * @param {NodeJS.ProcessEnv} [opts.baseEnv]
 * @param {boolean} [opts.needsCuda]
 * @param {string} [opts.backend] whisper-gpu backend: cuda | vulkan | metal | none
 * @param {string} [opts.logPrefix]
 * @returns {{
 *   env: NodeJS.ProcessEnv,
 *   cudaBuildEnv: ReturnType<typeof applyWindowsCudaBuildEnvIfNeeded>,
 *   generator: string | null,
 *   gpuPrebuild: ReturnType<typeof applyGpuPrebuildEnv> | null,
 *   discovered: Record<string, string>,
 * }}
 */
export function resolveBuildEnvironment(opts) {
  const {
    jarvisRoot,
    channel,
    baseEnv = process.env,
    needsCuda = false,
    backend = "none",
    logPrefix = channel === "cargo" ? "cargo-win-env" : "whisper-gpu",
  } = opts;

  const useCuda =
    process.platform === "win32" &&
    (needsCuda || backend === "cuda");

  /** @type {NodeJS.ProcessEnv} */
  const env = { ...baseEnv };
  env.CARGO_TERM_PROGRESS = env.CARGO_TERM_PROGRESS ?? "always";

  let cudaBuildEnv = null;
  let gpuPrebuild = null;

  if (useCuda) {
    if (channel !== "sync" && channel !== "prebuild") {
      applyDiscoveredCudaToolkitToProcessEnv();
    }
    cudaBuildEnv = applyWindowsCudaBuildEnvIfNeeded(env, {
      logPrefix,
      skipCompilerCache: channel === "prebuild",
    });
    if (cudaBuildEnv) {
      if (channel === "tauri" || channel === "cargo") {
        gpuPrebuild = applyGpuPrebuildEnv(env, jarvisRoot, { logPrefix });
      }
      if (channel !== "sync") {
        const cudaBin = path.join(cudaBuildEnv.cudaRoot, "bin");
        const cudaBinX64 = path.join(cudaBuildEnv.cudaRoot, "bin", "x64");
        const nvtxBin = path.join(cudaBuildEnv.cudaRoot, "extras", "CUPTI", "lib64");
        const existingPath = env.PATH ?? env.Path ?? "";
        const mergedPath = prependWindowsPathEntries(existingPath, [cudaBinX64, cudaBin, nvtxBin]);
        env.PATH = mergedPath;
        env.Path = mergedPath;
      }
    }
  } else if (process.platform === "win32" && !env.CMAKE_GENERATOR?.trim()) {
    env.CMAKE_GENERATOR = "Visual Studio 17 2022";
  }

  /** @type {Record<string, string>} */
  const discovered = {};

  if (process.platform === "win32") {
    if (channel !== "sync" && channel !== "prebuild") {
      assertWindowsWhisperBindgenEnv(logPrefix);
    }
    const whisperEnv = buildWindowsWhisperCargoEnv(env, {
      force: true,
      includeCmakeGenerator: !(useCuda && env.CMAKE_GENERATOR?.trim()),
    });
    Object.assign(env, whisperEnv);
    Object.assign(discovered, whisperEnv);

    if (useCuda && cudaBuildEnv?.generator) {
      discovered.CMAKE_GENERATOR = cudaBuildEnv.generator;
      if (env.CMAKE_MAKE_PROGRAM) {
        discovered.CMAKE_MAKE_PROGRAM = env.CMAKE_MAKE_PROGRAM;
      }
    } else if (!discovered.CMAKE_GENERATOR) {
      discovered.CMAKE_GENERATOR = env.CMAKE_GENERATOR ?? "Visual Studio 17 2022";
    }
  }

  if (useCuda === false && backend === "vulkan" && process.platform === "win32" && channel !== "sync") {
    const vkRoot = resolveWindowsVulkanSdkRoot();
    if (vkRoot) {
      env.VULKAN_SDK = vkRoot;
    }
  }

  const generator =
    cudaBuildEnv?.generator ??
    env.CMAKE_GENERATOR?.trim() ??
    discovered.CMAKE_GENERATOR ??
    null;

  return { env, cudaBuildEnv, generator, gpuPrebuild, discovered };
}

export function prepareGpuNativeBuild(jarvisRoot, env, opts = {}) {
  const logPrefix = opts.logPrefix ?? "whisper-gpu";
  const profile = opts.profile ?? "debug";
  const generator = opts.generator ?? env.CMAKE_GENERATOR ?? "NMake Makefiles";

  if (opts.warnGeneratorMismatch !== false) {
    warnIfCmakeGeneratorMismatch(jarvisRoot, generator);
  }

  const unusable = clearUnusableGpuSysCmakeCaches(jarvisRoot, profile);
  const launcher = purgeWindowsStaleCudaCompilerLauncherCaches(jarvisRoot, { logPrefix, profile });
  const purged = [...unusable.cleared, ...launcher.purged];

  if (env.JARVIS_GPU_PREBUILD_WARM !== "1") {
    console.warn(
      `${logPrefix}: GPU prebuild cache not warm — run: npm run prebuild:gpu-cuda (then retry). Full CUDA compile may take 45–90+ min.`,
    );
    return { seeded: [], purged, applied: 0, cacheWarm: false };
  }

  const seed = seedGpuPrebuildCache(jarvisRoot, {
    arch: env.JARVIS_GPU_PREBUILD_ARCH,
    generator,
    profile,
    env,
    log: { logPrefix },
  });

  for (const w of seed.warnings) {
    console.warn(`${logPrefix}: ${w}`);
  }

  const applied = seed.seeded.length;
  if (applied > 0) {
    if (process.platform === "win32") {
      delete env.CMAKE_C_COMPILER_LAUNCHER;
      delete env.CMAKE_CXX_COMPILER_LAUNCHER;
      delete env.CMAKE_CUDA_COMPILER_LAUNCHER;
    }
    console.log(`${logPrefix}: applied GPU prebuild cache to ${applied} -sys CMake tree(s)`);
  }

  return { seeded: seed.seeded, purged, applied, cacheWarm: true };
}

/**
 * @param {ReturnType<typeof applyWindowsCudaBuildEnvIfNeeded>} cudaBuildEnv
 * @param {ReturnType<typeof applyGpuPrebuildEnv> | null} gpuPrebuild
 */
export function formatResolvedCudaLog(cudaBuildEnv, gpuPrebuild) {
  if (!cudaBuildEnv) return null;
  return formatCudaBuildProfileLog({
    ...cudaBuildEnv,
    gpuPrebuildWarm: gpuPrebuild?.warm ?? false,
  });
}
