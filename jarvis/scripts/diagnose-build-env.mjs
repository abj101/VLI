/**
 * Read-only GPU build env diagnostic: generators, CUDA paths, ggml-cuda cache, mismatches.
 */

import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";

import {
  applyWindowsCudaBuildEnvIfNeeded,
  findCompilerCacheOnPath,
  findWindowsNinjaExe,
  resolveCudaArchitectures,
  resolveCudaBuildParallelLevel,
  resolveCudaCmakeGenerator,
  resolveJarvisCompilerCacheDir,
  resolveWindowsCudaToolkitRoot,
} from "./whisper-gpu/detect.mjs";
import { summarizeGpuPrebuildCache } from "./gpu-prebuild.mjs";
import {
  listGpuSysCmakeGeneratorMismatches,
  summarizeGgmlCudaArtifacts,
} from "./whisper-gpu/preflight.mjs";

const JARVIS_ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");

/**
 * @param {string} filePath
 * @returns {string | null}
 */
function readTomlEnvVar(filePath, key) {
  if (!fs.existsSync(filePath)) return null;
  try {
    const text = fs.readFileSync(filePath, "utf8");
    const re = new RegExp(`^${key}\\s*=\\s*"(.*)"\\s*$`, "m");
    const m = text.match(re);
    return m?.[1]?.trim() || null;
  } catch {
    return null;
  }
}

/**
 * @param {"debug"|"release"} profile
 */
export function collectBuildEnvDiagnostics(jarvisRoot, profile = "debug") {
  const configLocalPath = path.join(jarvisRoot, "src-tauri", ".cargo", "config.local.toml");
  const shellGenerator = process.env.CMAKE_GENERATOR?.trim() || null;
  const configLocalGenerator = readTomlEnvVar(configLocalPath, "CMAKE_GENERATOR");
  const cudaArch = resolveCudaArchitectures(process.env);
  const parallelLevel = resolveCudaBuildParallelLevel();

  /** @type {Record<string, string | null>} */
  const cudaTauriEnv = { ...process.env };
  const cudaApplied = applyWindowsCudaBuildEnvIfNeeded(cudaTauriEnv);
  const tauriCudaGenerator = cudaApplied?.generator ?? resolveCudaCmakeGenerator(cudaTauriEnv);

  const cudaPath =
    cudaTauriEnv.CUDA_PATH?.trim() ||
    resolveWindowsCudaToolkitRoot() ||
    process.env.CUDA_PATH?.trim() ||
    null;

  const artifacts = summarizeGgmlCudaArtifacts(jarvisRoot, profile);
  const gpuPrebuild = summarizeGpuPrebuildCache(jarvisRoot, cudaArch);
  const mismatchesTauri = listGpuSysCmakeGeneratorMismatches(
    jarvisRoot,
    tauriCudaGenerator,
    profile,
  );
  const mismatchesIde = configLocalGenerator
    ? listGpuSysCmakeGeneratorMismatches(jarvisRoot, configLocalGenerator, profile)
    : [];

  return {
    platform: process.platform,
    profile,
    shell: {
      CMAKE_GENERATOR: shellGenerator,
      CUDA_PATH: process.env.CUDA_PATH?.trim() || null,
      CMAKE_CUDA_ARCHITECTURES: process.env.CMAKE_CUDA_ARCHITECTURES?.trim() || null,
      JARVIS_CUDA_ARCH: process.env.JARVIS_CUDA_ARCH?.trim() || null,
    },
    configLocalToml: {
      path: path.relative(jarvisRoot, configLocalPath),
      CMAKE_GENERATOR: configLocalGenerator,
    },
    tauriCuda: {
      CMAKE_GENERATOR: tauriCudaGenerator,
      CUDA_PATH: cudaPath,
      CMAKE_CUDA_ARCHITECTURES: cudaTauriEnv.CMAKE_CUDA_ARCHITECTURES?.trim() || cudaArch,
      CMAKE_BUILD_PARALLEL_LEVEL:
        cudaTauriEnv.CMAKE_BUILD_PARALLEL_LEVEL?.trim() || parallelLevel,
      WHISPER_DONT_GENERATE_BINDINGS:
        cudaTauriEnv.WHISPER_DONT_GENERATE_BINDINGS?.trim() || null,
      CMAKE_C_COMPILER_LAUNCHER: cudaTauriEnv.CMAKE_C_COMPILER_LAUNCHER?.trim() || null,
      CCACHE_DIR: cudaTauriEnv.CCACHE_DIR?.trim() || null,
      SCCACHE_DIR: cudaTauriEnv.SCCACHE_DIR?.trim() || null,
      cudaEnvApplied: !!cudaApplied,
    },
    ggmlCudaArtifacts: artifacts.map(({ label, artifactPath }) => ({
      crate: label,
      present: !!artifactPath,
      path: artifactPath ? path.relative(jarvisRoot, artifactPath) : null,
    })),
    gpuPrebuild: {
      warm: gpuPrebuild.warm,
      cacheRoot: path.relative(jarvisRoot, gpuPrebuild.cacheRoot),
      manifest: gpuPrebuild.manifest,
      whisper: {
        warm: gpuPrebuild.entries.whisper.warm,
        path: gpuPrebuild.entries.whisper.artifact
          ? path.relative(jarvisRoot, gpuPrebuild.entries.whisper.artifact)
          : null,
      },
      llama: {
        warm: gpuPrebuild.entries.llama.warm,
        path: gpuPrebuild.entries.llama.artifact
          ? path.relative(jarvisRoot, gpuPrebuild.entries.llama.artifact)
          : null,
      },
    },
    generatorMismatches: {
      vsTauriCuda: mismatchesTauri.map(({ label, cached }) => ({
        crate: label,
        cached,
        intended: tauriCudaGenerator,
      })),
      vsConfigLocal: mismatchesIde.map(({ label, cached }) => ({
        crate: label,
        cached,
        intended: configLocalGenerator,
      })),
    },
    warnings: buildDiagnosticWarnings({
      shellGenerator,
      tauriCudaGenerator,
      configLocalGenerator,
      mismatchesTauri,
      mismatchesIde,
      cudaApplied,
    }),
  };
}

/**
 * @param {object} ctx
 */
function buildDiagnosticWarnings(ctx) {
  /** @type {string[]} */
  const warnings = [];

  if (
    ctx.shellGenerator &&
    ctx.shellGenerator !== ctx.tauriCudaGenerator &&
    process.platform === "win32"
  ) {
    warnings.push(
      `Shell CMAKE_GENERATOR="${ctx.shellGenerator}" differs from tauri CUDA target "${ctx.tauriCudaGenerator}" - npm run tauri * overrides at runtime, but bare cargo in this shell may use the stale value.`,
    );
  }

  if (/visual studio/i.test(ctx.shellGenerator ?? "")) {
    warnings.push(
      "Remove global/user CMAKE_GENERATOR=Visual Studio from Windows env - it triggers full GPU -sys rebuilds when mixed with tauri CUDA (NMake/Ninja).",
    );
  }

  if (ctx.configLocalGenerator && ctx.configLocalGenerator !== ctx.tauriCudaGenerator) {
    warnings.push(
      `config.local.toml uses "${ctx.configLocalGenerator}" while tauri CUDA uses "${ctx.tauriCudaGenerator}" - rust-analyzer / bare cargo check can invalidate whisper-rs-sys and llama-cpp-sys CMake caches.`,
    );
  }

  if (ctx.mismatchesTauri.length > 0) {
    warnings.push(
      `${ctx.mismatchesTauri.length} GPU -sys CMake cache(s) disagree with tauri CUDA generator "${ctx.tauriCudaGenerator}".`,
    );
  }

  if (ctx.mismatchesIde.length > 0) {
    warnings.push(
      `${ctx.mismatchesIde.length} GPU -sys CMake cache(s) disagree with config.local.toml generator "${ctx.configLocalGenerator}".`,
    );
  }

  if (process.platform === "win32" && !ctx.cudaApplied) {
    warnings.push(
      "CUDA build env could not be applied (missing CUDA toolkit, nvcc, or MSVC NMake host).",
    );
  }

  return warnings;
}

function printDiagnostics(report) {
  console.log("jarvis build-env diagnostic");
  console.log(`platform: ${report.platform}  profile: ${report.profile}`);
  console.log("");

  console.log("Shell / process env:");
  console.log(`  CMAKE_GENERATOR: ${report.shell.CMAKE_GENERATOR ?? "(unset)"}`);
  console.log(`  CUDA_PATH: ${report.shell.CUDA_PATH ?? "(unset)"}`);
  console.log(
    `  CMAKE_CUDA_ARCHITECTURES: ${report.shell.CMAKE_CUDA_ARCHITECTURES ?? "(unset)"}`,
  );
  console.log(`  JARVIS_CUDA_ARCH: ${report.shell.JARVIS_CUDA_ARCH ?? "(unset)"}`);
  console.log(`  resolved arch pin (tauri default): ${resolveCudaArchitectures(process.env)}`);
  console.log("");

  console.log(`${report.configLocalToml.path} (bare cargo / rust-analyzer):`);
  console.log(`  CMAKE_GENERATOR: ${report.configLocalToml.CMAKE_GENERATOR ?? "(unset)"}`);
  console.log("");

  if (process.platform === "win32") {
    const ninja = findWindowsNinjaExe();
    console.log(`ninja.exe on PATH: ${ninja ?? "no (install: winget install Ninja-build.Ninja)"}`);
    const compilerCache = findCompilerCacheOnPath();
    if (compilerCache) {
      console.log(
        `compiler cache on PATH: ${compilerCache.launcher} (${compilerCache.exe}); cache dir ${resolveJarvisCompilerCacheDir(compilerCache.launcher)}`,
      );
    } else {
      console.log(
        "compiler cache on PATH: no (optional: winget install Ccache.Ccache or winget install Mozilla.sccache)",
      );
    }
    console.log("");
  }

  console.log("npm run tauri * CUDA path (after detect.mjs):");
  console.log(`  CMAKE_GENERATOR: ${report.tauriCuda.CMAKE_GENERATOR}`);
  console.log(`  CUDA_PATH: ${report.tauriCuda.CUDA_PATH ?? "(unset)"}`);
  console.log(
    `  CMAKE_CUDA_ARCHITECTURES: ${report.tauriCuda.CMAKE_CUDA_ARCHITECTURES ?? "(unset)"}`,
  );
  console.log(
    `  CMAKE_BUILD_PARALLEL_LEVEL: ${report.tauriCuda.CMAKE_BUILD_PARALLEL_LEVEL ?? "(unset)"}`,
  );
  console.log(
    `  WHISPER_DONT_GENERATE_BINDINGS: ${report.tauriCuda.WHISPER_DONT_GENERATE_BINDINGS ?? "(unset)"}`,
  );
  console.log(
    `  CMAKE_C_COMPILER_LAUNCHER: ${report.tauriCuda.CMAKE_C_COMPILER_LAUNCHER ?? "(unset)"}`,
  );
  console.log(`  CCACHE_DIR: ${report.tauriCuda.CCACHE_DIR ?? "(unset)"}`);
  console.log(`  SCCACHE_DIR: ${report.tauriCuda.SCCACHE_DIR ?? "(unset)"}`);
  console.log(`  cuda env applied: ${report.tauriCuda.cudaEnvApplied ? "yes" : "no"}`);
  console.log("");

  console.log("ggml-cuda artifacts:");
  for (const row of report.ggmlCudaArtifacts) {
    console.log(
      `  ${row.crate}: ${row.present ? row.path : "not built yet"}`,
    );
  }
  console.log("");

  console.log("GPU prebuild cache (.cache/gpu-prebuild):");
  console.log(`  warm: ${report.gpuPrebuild.warm ? "yes" : "no"}`);
  console.log(`  root: ${report.gpuPrebuild.cacheRoot}`);
  if (report.gpuPrebuild.manifest) {
    console.log(
      `  manifest: arch=${report.gpuPrebuild.manifest.arch} generator=${report.gpuPrebuild.manifest.generator}`,
    );
  } else {
    console.log("  manifest: (missing — run npm run prebuild:gpu-cuda)");
  }
  console.log(
    `  whisper: ${report.gpuPrebuild.whisper.warm ? report.gpuPrebuild.whisper.path : "not built"}`,
  );
  console.log(
    `  llama: ${report.gpuPrebuild.llama.warm ? report.gpuPrebuild.llama.path : "not built"}`,
  );
  console.log("");

  const allMismatches = [
    ...report.generatorMismatches.vsTauriCuda.map((m) => ({
      ...m,
      source: "tauri CUDA",
    })),
    ...report.generatorMismatches.vsConfigLocal.map((m) => ({
      ...m,
      source: "config.local.toml",
    })),
  ];
  if (allMismatches.length === 0) {
    console.log("CMake generator mismatches: none detected");
  } else {
    console.log("CMake generator mismatches:");
    for (const m of allMismatches) {
      console.log(
        `  ${m.crate} [${m.source}]: cached "${m.cached}" vs intended "${m.intended}"`,
      );
    }
  }

  if (report.warnings.length > 0) {
    console.log("");
    console.log("Warnings:");
    for (const w of report.warnings) {
      console.log(`  - ${w}`);
    }
  }
}

function main() {
  const profile = process.argv.includes("--release") ? "release" : "debug";
  const report = collectBuildEnvDiagnostics(JARVIS_ROOT, profile);
  printDiagnostics(report);
}

const __filename = fileURLToPath(import.meta.url);
if (process.argv[1] && path.resolve(process.argv[1]) === __filename) {
  main();
}
