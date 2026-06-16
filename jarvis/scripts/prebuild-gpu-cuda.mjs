#!/usr/bin/env node
/**
 * One-shot CMake Release CUDA prebuild for whisper.cpp + llama.cpp (ggml-cuda).
 * Cache: jarvis/.cache/gpu-prebuild/<arch>/ — seeded into cargo `-sys` trees when warm.
 *
 * Usage (from jarvis/):
 *   npm run prebuild:gpu-cuda
 *   node scripts/prebuild-gpu-cuda.mjs --force
 *   node scripts/prebuild-gpu-cuda.mjs --arch 86
 */

import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";

import { formatResolvedCudaLog, resolveBuildEnvironment } from "./build-environment.mjs";
import {
  hasCudaToolchain,
  resolveCudaArchitectures,
} from "./whisper-gpu/detect.mjs";
import {
  isGpuPrebuildCacheWarm,
  llamaCudaPrebuildDefines,
  readCargoLockSysVersions,
  resolveCargoRegistryCrateDir,
  resolveGpuPrebuildCacheRoot,
  resolveGpuPrebuildEntryDirs,
  runCmakeReleasePrebuild,
  seedGpuPrebuildCache,
  syncSourceTreeForPrebuild,
  whisperCudaPrebuildDefines,
  writeGpuPrebuildManifest,
} from "./gpu-prebuild.mjs";

const JARVIS_ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");

function parseArgs(argv) {
  let force = false;
  let archOverride = null;
  let skipSeed = false;

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--force") {
      force = true;
    } else if (arg === "--skip-seed") {
      skipSeed = true;
    } else if (arg === "--arch" && argv[i + 1]) {
      archOverride = argv[++i].trim();
    } else if (arg === "-h" || arg === "--help") {
      console.log(`Usage: node scripts/prebuild-gpu-cuda.mjs [--force] [--arch 89] [--skip-seed]

Builds whisper.cpp + llama.cpp CUDA (Release, pinned arch) into jarvis/.cache/gpu-prebuild/<arch>/.
Requires Windows + CUDA toolkit + MSVC (same as npm run tauri:dev:gpu).

After prebuild, npm run tauri * / npm run cargo with whisper-cuda+llm-cuda seeds warm cache into
cargo -sys out/build dirs (ccache/sccache also speeds repeat nvcc when enabled).`);
      process.exit(0);
    }
  }

  return { force, archOverride, skipSeed };
}

function ensureWindowsPlatform() {
  if (process.platform !== "win32") {
    console.error("prebuild-gpu-cuda: Windows + CUDA only (matches jarvis GPU dev path).");
    process.exit(1);
  }
}

function resolveRegistrySources(versions) {
  const whisperCrate = resolveCargoRegistryCrateDir("whisper-rs-sys", versions.whisperRsSys);
  const llamaCrate = resolveCargoRegistryCrateDir("llama-cpp-sys-2", versions.llamaCppSys2);
  if (!whisperCrate || !llamaCrate) {
    console.error(
      "prebuild-gpu-cuda: crate sources not in local registry. Run from jarvis/:\n" +
        "  npm run cargo -- fetch --manifest-path src-tauri/Cargo.toml",
    );
    process.exit(1);
  }

  const whisperSource = path.join(whisperCrate, "whisper.cpp");
  const llamaSource = path.join(llamaCrate, "llama.cpp");
  if (!fs.existsSync(whisperSource) || !fs.existsSync(llamaSource)) {
    console.error("prebuild-gpu-cuda: whisper.cpp / llama.cpp not found in registry crates.");
    process.exit(1);
  }

  return { whisperSource, llamaSource, whisperCrate, llamaCrate };
}

async function main() {
  const { force, archOverride, skipSeed } = parseArgs(process.argv.slice(2));
  ensureWindowsPlatform();

  /** @type {NodeJS.ProcessEnv} */
  const baseEnv = { ...process.env };
  if (archOverride) {
    baseEnv.JARVIS_CUDA_ARCH = archOverride;
  }
  const arch = resolveCudaArchitectures();

  const versions = readCargoLockSysVersions(JARVIS_ROOT);
  if (!versions) {
    console.error("prebuild-gpu-cuda: could not read whisper-rs-sys / llama-cpp-sys-2 from Cargo.lock");
    process.exit(1);
  }

  const cacheRoot = resolveGpuPrebuildCacheRoot(JARVIS_ROOT, arch);
  const { env, cudaBuildEnv } = resolveBuildEnvironment({
    jarvisRoot: JARVIS_ROOT,
    channel: "prebuild",
    baseEnv,
    needsCuda: true,
    logPrefix: "prebuild-gpu-cuda",
  });
  if (!cudaBuildEnv || !hasCudaToolchain()) {
    console.error(
      "prebuild-gpu-cuda: CUDA toolkit not found. Install CUDA or set CUDA_PATH. See jarvis/README.md.",
    );
    process.exit(1);
  }
  const cudaLog = formatResolvedCudaLog(cudaBuildEnv, null);
  if (cudaLog) console.log(`prebuild-gpu-cuda: ${cudaLog}`);
  console.log(`prebuild-gpu-cuda: cache root ${cacheRoot}`);

  const generator = env.CMAKE_GENERATOR ?? cudaBuildEnv.generator;
  if (!force && isGpuPrebuildCacheWarm(JARVIS_ROOT, arch, generator)) {
    console.log(
      `prebuild-gpu-cuda: cache already warm for arch=${arch} generator=${generator} (use --force to rebuild)`,
    );
  } else {
    const { whisperSource, llamaSource } = resolveRegistrySources(versions);

    const whisperDirs = resolveGpuPrebuildEntryDirs("whisper", cacheRoot);
    const llamaDirs = resolveGpuPrebuildEntryDirs("llama", cacheRoot);

    console.log("prebuild-gpu-cuda: syncing whisper.cpp sources into cache…");
    syncSourceTreeForPrebuild(whisperSource, whisperDirs.sourceDir);
    console.log("prebuild-gpu-cuda: syncing llama.cpp sources into cache…");
    syncSourceTreeForPrebuild(llamaSource, llamaDirs.sourceDir);

    runCmakeReleasePrebuild({
      label: "whisper",
      sourceDir: whisperDirs.sourceDir,
      buildDir: whisperDirs.buildDir,
      env,
      defines: whisperCudaPrebuildDefines(),
    });

    runCmakeReleasePrebuild({
      label: "llama",
      sourceDir: llamaDirs.sourceDir,
      buildDir: llamaDirs.buildDir,
      env,
      defines: llamaCudaPrebuildDefines(),
    });

    writeGpuPrebuildManifest(cacheRoot, {
      arch,
      generator,
      profile: "Release",
      whisperRsSysVersion: versions.whisperRsSys,
      llamaCppSysVersion: versions.llamaCppSys2,
      createdAt: new Date().toISOString(),
      platform: process.platform,
    });

    console.log(`prebuild-gpu-cuda: wrote ${path.join(cacheRoot, "manifest.json")}`);
  }

  if (!skipSeed) {
    const seed = seedGpuPrebuildCache(JARVIS_ROOT, {
      arch,
      generator,
      profile: "debug",
      force,
      log: { logPrefix: "prebuild-gpu-cuda" },
    });
    if (seed.seeded.length > 0) {
      console.log(`prebuild-gpu-cuda: seeded ${seed.seeded.length} cargo out/build dir(s)`);
    }
    for (const line of seed.skipped) {
      console.log(`prebuild-gpu-cuda: seed skipped — ${line}`);
    }
    for (const w of seed.warnings) {
      console.warn(`prebuild-gpu-cuda: ${w}`);
    }
  }

  console.log(
    "prebuild-gpu-cuda: done. Next: npm run tauri:dev:gpu (or npm run cargo -- check … whisper-cuda,llm-cuda)",
  );
}

main().catch((err) => {
  console.error("prebuild-gpu-cuda:", err instanceof Error ? err.message : err);
  process.exit(1);
});
