/**
 * GPU CUDA prebuild cache — standalone CMake trees for whisper.cpp + llama.cpp.
 * Populated by `prebuild-gpu-cuda.mjs`; seeded into cargo `-sys` out/build dirs when warm.
 */

import fs from "fs";
import os from "os";
import path from "path";
import { spawnSync } from "child_process";

import {
  applyCompilerCacheLauncherEnv,
  applyCudaBuildTuningEnv,
  applyCudaCmakeGenerator,
  resolveCudaArchitectures,
  resolveCudaBuildParallelLevel,
  resolveCudaCmakeGenerator,
} from "./whisper-gpu/detect.mjs";
import {
  GPU_NATIVE_BUILD_PREFIXES,
  isCmakeCacheWellFormed,
  readCmakeCacheValue,
  repairCmakeCacheMalformedEntries,
} from "./gpu-native-sys.mjs";

export const MANIFEST_FILE = "manifest.json";

/**
 * @param {string} text
 * @param {string} key
 * @param {string} value
 */
function replaceCmakeCacheValue(text, key, value) {
  const patterns = [
    new RegExp(`^(${key}:INTERNAL=).*$`, "m"),
    new RegExp(`^(${key}:PATH=).*$`, "m"),
    new RegExp(`^(${key}:STRING=).*$`, "m"),
  ];
  for (const re of patterns) {
    if (re.test(text)) {
      return text.replace(re, `$1${value}`);
    }
  }
  return `${text.trimEnd()}\n${key}:INTERNAL=${value}\n`;
}

/** @typedef {"whisper" | "llama"} GpuPrebuildEntryId */

/**
 * @typedef {object} GpuPrebuildManifest
 * @property {string} arch
 * @property {string} generator
 * @property {string} profile
 * @property {string} whisperRsSysVersion
 * @property {string} llamaCppSysVersion
 * @property {string} createdAt
 * @property {string} [platform]
 */

/**
 * @param {string} jarvisRoot
 * @param {string} [arch]
 */
export function resolveGpuPrebuildCacheRoot(jarvisRoot, arch = resolveCudaArchitectures()) {
  return path.join(jarvisRoot, ".cache", "gpu-prebuild", arch);
}

/**
 * @param {GpuPrebuildEntryId} entryId
 * @param {string} cacheRoot
 */
export function resolveGpuPrebuildEntryDirs(entryId, cacheRoot) {
  const entryRoot = path.join(cacheRoot, entryId);
  return {
    entryRoot,
    sourceDir: path.join(entryRoot, "source"),
    buildDir: path.join(entryRoot, "build"),
  };
}

/**
 * @param {string} jarvisRoot
 * @returns {{ whisperRsSys: string, llamaCppSys2: string } | null}
 */
export function readCargoLockSysVersions(jarvisRoot) {
  const lockPath = path.join(jarvisRoot, "src-tauri", "Cargo.lock");
  if (!fs.existsSync(lockPath)) return null;

  let text;
  try {
    text = fs.readFileSync(lockPath, "utf8");
  } catch {
    return null;
  }

  const whisperBlock = text.match(
    /\[\[package\]\]\s*\nname = "whisper-rs-sys"\s*\nversion = "([^"]+)"/,
  );
  const llamaBlock = text.match(
    /\[\[package\]\]\s*\nname = "llama-cpp-sys-2"\s*\nversion = "([^"]+)"/,
  );
  if (!whisperBlock?.[1] || !llamaBlock?.[1]) return null;

  return {
    whisperRsSys: whisperBlock[1],
    llamaCppSys2: llamaBlock[1],
  };
}

/**
 * @param {string} crateName e.g. whisper-rs-sys
 * @param {string} version
 * @returns {string | null}
 */
export function resolveCargoRegistryCrateDir(crateName, version) {
  const cargoHome = process.env.CARGO_HOME?.trim() || path.join(os.homedir(), ".cargo");
  const registryRoot = path.join(
    cargoHome,
    "registry",
    "src",
    "index.crates.io-1949cf8c6b5b557f",
  );
  const exact = path.join(registryRoot, `${crateName}-${version}`);
  if (fs.existsSync(exact)) return exact;

  if (!fs.existsSync(registryRoot)) return null;
  const dirName = fs
    .readdirSync(registryRoot)
    .find((name) => name === `${crateName}-${version}` || name.startsWith(`${crateName}-`));
  return dirName ? path.join(registryRoot, dirName) : null;
}

/**
 * @param {string} buildDir
 * @returns {string | null}
 */
export function findGgmlCudaLibInBuildDir(buildDir) {
  if (!buildDir || !fs.existsSync(buildDir)) return null;

  const markers = [
    path.join(buildDir, "ggml", "src", "ggml-cuda", "ggml-cuda.lib"),
    path.join(buildDir, "lib", "ggml-cuda.lib"),
    path.join(buildDir, "ggml", "src", "ggml-cuda", "libggml-cuda.a"),
  ];
  for (const marker of markers) {
    if (fs.existsSync(marker)) return marker;
  }

  const walk = (dir, depth) => {
    if (depth > 8) return null;
    let entries;
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
      return null;
    }
    for (const ent of entries) {
      const p = path.join(dir, ent.name);
      if (ent.isFile() && /^ggml-cuda\.(lib|a)$/i.test(ent.name)) return p;
      if (ent.isDirectory()) {
        const nested = walk(p, depth + 1);
        if (nested) return nested;
      }
    }
    return null;
  };
  return walk(buildDir, 0);
}

/**
 * @param {string} buildDir
 */
export function isGpuPrebuildEntryWarm(buildDir) {
  return !!findGgmlCudaLibInBuildDir(buildDir);
}

/**
 * @param {string} cacheRoot
 * @returns {GpuPrebuildManifest | null}
 */
export function readGpuPrebuildManifest(cacheRoot) {
  const manifestPath = path.join(cacheRoot, MANIFEST_FILE);
  if (!fs.existsSync(manifestPath)) return null;
  try {
    return /** @type {GpuPrebuildManifest} */ (JSON.parse(fs.readFileSync(manifestPath, "utf8")));
  } catch {
    return null;
  }
}

/**
 * @param {string} cacheRoot
 * @param {GpuPrebuildManifest} manifest
 */
export function writeGpuPrebuildManifest(cacheRoot, manifest) {
  fs.mkdirSync(cacheRoot, { recursive: true });
  fs.writeFileSync(
    path.join(cacheRoot, MANIFEST_FILE),
    `${JSON.stringify(manifest, null, 2)}\n`,
    "utf8",
  );
}

/**
 * @param {string} jarvisRoot
 * @param {string} [arch]
 * @param {string} [generator]
 */
export function isGpuPrebuildCacheWarm(jarvisRoot, arch = resolveCudaArchitectures(), generator) {
  const cacheRoot = resolveGpuPrebuildCacheRoot(jarvisRoot, arch);
  const manifest = readGpuPrebuildManifest(cacheRoot);
  const intendedGenerator = generator ?? resolveCudaCmakeGenerator();
  if (!manifest || manifest.arch !== arch) return false;
  if (manifest.generator && manifest.generator !== intendedGenerator) return false;

  const lockVersions = readCargoLockSysVersions(jarvisRoot);
  if (lockVersions) {
    if (manifest.whisperRsSysVersion !== lockVersions.whisperRsSys) return false;
    if (manifest.llamaCppSysVersion !== lockVersions.llamaCppSys2) return false;
  }

  const whisper = resolveGpuPrebuildEntryDirs("whisper", cacheRoot);
  const llama = resolveGpuPrebuildEntryDirs("llama", cacheRoot);
  return isGpuPrebuildEntryWarm(whisper.buildDir) && isGpuPrebuildEntryWarm(llama.buildDir);
}

/**
 * @param {NodeJS.ProcessEnv} envObj
 * @param {string} jarvisRoot
 * @param {{ logPrefix?: string }} [opts]
 */
export function applyGpuPrebuildEnv(envObj, jarvisRoot, opts = {}) {
  const logPrefix = opts.logPrefix ?? "gpu-prebuild";
  const arch = resolveCudaArchitectures(envObj);
  const generator = resolveCudaCmakeGenerator(envObj);
  const cacheRoot = resolveGpuPrebuildCacheRoot(jarvisRoot, arch);

  envObj.JARVIS_GPU_PREBUILD_ROOT = cacheRoot;
  envObj.JARVIS_GPU_PREBUILD_ARCH = arch;
  // llama-cpp-sys uses always_configure(false); force static ggml when seeding warm cache.
  envObj.LLAMA_BUILD_SHARED_LIBS = "0";

  const warm = isGpuPrebuildCacheWarm(jarvisRoot, arch, generator);
  if (warm) {
    envObj.JARVIS_GPU_PREBUILD_WARM = "1";
    const whisper = resolveGpuPrebuildEntryDirs("whisper", cacheRoot);
    const llama = resolveGpuPrebuildEntryDirs("llama", cacheRoot);
    envObj.JARVIS_WHISPER_PREBUILD_BUILD_DIR = whisper.buildDir;
    envObj.JARVIS_LLAMA_PREBUILD_BUILD_DIR = llama.buildDir;
  } else {
    delete envObj.JARVIS_GPU_PREBUILD_WARM;
    delete envObj.JARVIS_WHISPER_PREBUILD_BUILD_DIR;
    delete envObj.JARVIS_LLAMA_PREBUILD_BUILD_DIR;
  }

  return { cacheRoot, arch, generator, warm };
}

/**
 * @param {string} fromDir
 * @param {string} toDir
 */
export function copyDirRecursive(fromDir, toDir) {
  fs.mkdirSync(path.dirname(toDir), { recursive: true });
  if (fs.existsSync(toDir)) {
    fs.rmSync(toDir, { recursive: true, force: true });
  }
  fs.cpSync(fromDir, toDir, { recursive: true, force: true });
}

/**
 * @param {string} buildDir
 * @param {string} newHomeDir
 */
export function rewriteCmakeHomeDirectory(buildDir, newHomeDir) {
  return rewriteSeededCmakeCache(buildDir, { homeDir: newHomeDir });
}

/**
 * Fix CMakeCache paths after copying a prebuild tree into cargo `-sys` out/build.
 * @param {string} buildDir
 * @param {{ homeDir: string }} opts
 */
export function rewriteSeededCmakeCache(buildDir, { homeDir }) {
  const cachePath = path.join(buildDir, "CMakeCache.txt");
  if (!fs.existsSync(cachePath)) return false;

  const home = path.resolve(homeDir).replace(/\\/g, "/");
  const cacheFile = path.resolve(buildDir).replace(/\\/g, "/");
  const installPrefix = path.resolve(buildDir, "..").replace(/\\/g, "/");
  let text = fs.readFileSync(cachePath, "utf8");
  text = repairCmakeCacheMalformedEntries(text);
  let changed = false;

  for (const [key, value] of [
    ["CMAKE_HOME_DIRECTORY", home],
    ["CMAKE_CACHEFILE_DIR", cacheFile],
    ["CMAKE_INSTALL_PREFIX", installPrefix],
    ["BUILD_SHARED_LIBS", "OFF"],
  ]) {
    const re = new RegExp(`^(${key}:(?:INTERNAL|PATH|STRING|BOOL)=).*$`, "m");
    if (re.test(text)) {
      text = text.replace(re, `$1${value}`);
    } else {
      text = `${text.trimEnd()}\n${key}:INTERNAL=${value}\n`;
    }
    changed = true;
  }

  if (process.platform === "win32") {
    const stripped = stripWindowsCompilerLaunchersFromCmakeCache(text);
    if (stripped !== text) {
      text = stripped;
      changed = true;
    }
  }

  if (changed) {
    fs.writeFileSync(cachePath, text, "utf8");
  }
  return changed;
}

/** @param {string} cacheText */
function stripWindowsCompilerLaunchersFromCmakeCache(cacheText) {
  const launcherKeys = [
    "CMAKE_C_COMPILER_LAUNCHER",
    "CMAKE_CXX_COMPILER_LAUNCHER",
    "CMAKE_CUDA_COMPILER_LAUNCHER",
  ];
  let text = cacheText;
  for (const key of launcherKeys) {
    text = text.replace(new RegExp(`^${key}:.*(?:\r?\n|$)`, "gm"), "");
  }
  return text.replace(/\n{3,}/g, "\n\n");
}

/** @param {string} absPath */
function pathPrefixVariants(absPath) {
  const resolved = path.resolve(absPath);
  const forward = resolved.replace(/\\/g, "/");
  const lower = forward.toLowerCase();
  const ninjaForward = forward.replace(/^([A-Za-z]):/, "$1$:");
  const ninjaLower = lower.replace(/^([a-z]):/, "$1$:");
  /** @type {string[]} */
  const bases = [
    resolved,
    forward,
    lower,
    resolved.replace(/\\/g, "\\\\"),
    forward.replace(/\//g, "\\\\"),
    ninjaForward,
    ninjaLower,
  ];
  /** @type {Set<string>} */
  const variants = new Set();
  for (const base of bases) {
    if (base.length <= 3) continue;
    variants.add(base);
    variants.add(`${base}/`);
    variants.add(`${base}\\`);
  }
  // Ninja cmake_ninja_workdir uses C$: with backslashes and a trailing \
  for (const ninjaBase of [ninjaForward, ninjaLower]) {
    const ninjaBackslash = `${ninjaBase.replace(/\//g, "\\")}\\`;
    if (ninjaBackslash.length > 3) variants.add(ninjaBackslash);
  }
  return [...variants].sort((a, b) => b.length - a.length);
}

/**
 * Rewrite absolute paths in a copied prebuild CMake tree (build.ninja, cmake_install.cmake, etc.).
 * @param {string} buildDir
 * @param {Array<[string, string]>} pairs from → to absolute paths
 * @returns {number} files changed
 */
export function rewritePathPrefixesInSeededBuildTree(buildDir, pairs) {
  /** @type {Array<[string, string]>} */
  const replacements = [];
  for (const [from, to] of pairs) {
    if (!from?.trim() || !to?.trim()) continue;
    const toResolved = path.resolve(to);
    for (const fromVariant of pathPrefixVariants(from)) {
      let toVariant = toResolved;
      const fromEndsWithSep = /[/\\]$/.test(fromVariant);
      if (fromVariant.includes("/") && !fromVariant.includes("\\")) {
        toVariant = toResolved.replace(/\\/g, "/");
      } else if (fromVariant.includes("$:")) {
        let toForward = toResolved.replace(/\\/g, "/");
        if (!/^c\$:/i.test(toForward)) {
          toForward = toForward.replace(/^([A-Za-z]):/, "$1$:");
        }
        if (fromVariant.endsWith("\\")) {
          toVariant = `${toForward.replace(/\//g, "\\")}\\`;
        } else {
          toVariant = toForward;
        }
      }
      if (fromEndsWithSep && !/[/\\]$/.test(toVariant)) {
        toVariant += fromVariant.endsWith("\\") ? "\\" : "/";
      }
      replacements.push([fromVariant, toVariant]);
    }
  }
  replacements.sort((a, b) => b[0].length - a[0].length);

  const textExtensions = new Set([".txt", ".ninja", ".cmake", ".json", ".rsp"]);
  let filesChanged = 0;

  /** @param {string} dir */
  const walk = (dir) => {
    let entries;
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const ent of entries) {
      const full = path.join(dir, ent.name);
      if (ent.isDirectory()) {
        walk(full);
        continue;
      }
      if (ent.name === "CMakeCache.txt") continue;
      const ext = path.extname(ent.name).toLowerCase();
      if (!textExtensions.has(ext)) continue;
      let text;
      try {
        text = fs.readFileSync(full, "utf8");
      } catch {
        continue;
      }
      let next = text;
      for (const [from, to] of replacements) {
        if (from.length < 12 || !next.includes(from)) continue;
        next = next.split(from).join(to);
      }
      if (next !== text) {
        fs.writeFileSync(full, next, "utf8");
        filesChanged += 1;
      }
    }
  };

  walk(buildDir);
  return filesChanged;
}

/**
 * Resolve llama.cpp source dir for a seeded llama-cpp-sys CMake tree.
 * @param {string} jarvisRoot
 */
export function resolveLlamaCppSourceDir(jarvisRoot) {
  const versions = readCargoLockSysVersions(jarvisRoot);
  if (!versions) return null;
  const crateDir = resolveCargoRegistryCrateDir("llama-cpp-sys-2", versions.llamaCppSys2);
  return crateDir ? path.join(crateDir, "llama.cpp") : null;
}

/**
 * Remove shared ggml/llama DLLs from a seeded `-sys` OUT_DIR after switching to static libs.
 * @param {string} outDir cargo OUT_DIR (parent of build/)
 */
function removeSharedGgmlInstallArtifacts(outDir) {
  const binDir = path.join(outDir, "bin");
  if (!fs.existsSync(binDir)) return;
  for (const name of fs.readdirSync(binDir)) {
    if (!name.endsWith(".dll")) continue;
    if (name.startsWith("ggml") || name === "llama.dll") {
      fs.rmSync(path.join(binDir, name), { force: true });
    }
  }
}

/**
 * @param {string} buildDir
 * @param {NodeJS.ProcessEnv} env
 */
function runCmakeInstallOnly(buildDir, env) {
  const cmake = env.CMAKE?.trim() || "cmake";
  const install = spawnSync(
    cmake,
    ["--install", path.resolve(buildDir), "--config", "Release"],
    { env, encoding: "utf8", shell: false },
  );
  if (install.status !== 0) {
    const detail = (install.stderr || install.stdout || "").trim().slice(0, 400);
    console.warn(
      `gpu-prebuild: cmake install failed for ${path.basename(buildDir)}${detail ? `: ${detail}` : ""}`,
    );
    return false;
  }
  return true;
}

/**
 * @param {string} buildDir
 * @param {NodeJS.ProcessEnv} env
 */
function runCmakeBuildInstall(buildDir, env) {
  const cmake = env.CMAKE?.trim() || "cmake";
  const parallel = env.CMAKE_BUILD_PARALLEL_LEVEL?.trim() || resolveCudaBuildParallelLevel();
  const build = spawnSync(
    cmake,
    ["--build", path.resolve(buildDir), "--config", "Release", "--parallel", parallel],
    { env, encoding: "utf8", shell: false },
  );
  if (build.status !== 0) {
    const detail = (build.stderr || build.stdout || "").trim().slice(0, 400);
    console.warn(
      `gpu-prebuild: cmake build failed for ${path.basename(buildDir)}${detail ? `: ${detail}` : ""}`,
    );
    return false;
  }
  const install = spawnSync(
    cmake,
    ["--install", path.resolve(buildDir), "--config", "Release"],
    { env, encoding: "utf8", shell: false },
  );
  if (install.status !== 0) {
    const detail = (install.stderr || install.stdout || "").trim().slice(0, 400);
    console.warn(
      `gpu-prebuild: cmake install failed for ${path.basename(buildDir)}${detail ? `: ${detail}` : ""}`,
    );
    return false;
  }
  return true;
}

/**
 * Regenerate build.ninja in a seeded tree so CMAKE_BINARY_DIR matches out/build (not prebuild cache).
 * @param {string} buildDir
 * @param {string} sourceDir
 * @param {NodeJS.ProcessEnv} env
 * @param {GpuPrebuildEntryId} [entryId]
 */
export function regenerateSeededCmakeBuildNinja(buildDir, sourceDir, env, entryId = "llama") {
  const cachePath = path.join(buildDir, "CMakeCache.txt");
  if (!fs.existsSync(cachePath) || !fs.existsSync(sourceDir)) return false;

  // Copied prebuild CMakeCache can contain stale paths/types; configure with a clean cache file.
  try {
    fs.unlinkSync(cachePath);
  } catch {
    return false;
  }

  const cmake = env.CMAKE?.trim() || "cmake";
  const installPrefix = path.resolve(buildDir, "..").replace(/\\/g, "/");
  const defines =
    entryId === "whisper" ? whisperCudaPrebuildDefines() : llamaCudaPrebuildDefines();
  /** @type {string[]} */
  const configureArgs = [
    path.resolve(sourceDir),
    "-B",
    path.resolve(buildDir),
    `-DCMAKE_INSTALL_PREFIX=${installPrefix}`,
  ];
  for (const [key, value] of Object.entries(defines)) {
    configureArgs.push(`-D${key}=${value}`);
  }
  const result = spawnSync(cmake, configureArgs, { env, encoding: "utf8", shell: false });
  if (result.status !== 0) {
    const detail = (result.stderr || result.stdout || "").trim().slice(0, 400);
    console.warn(
      `gpu-prebuild: cmake regen failed for ${path.basename(buildDir)}${detail ? `: ${detail}` : ""}`,
    );
    return false;
  }
  return true;
}

/**
 * True when a warm seeded tree still references the gpu-prebuild cache in build.ninja / CMakeCache.
 * @param {string} buildDir
 */
export function seededCmakeBuildNeedsRegen(buildDir) {
  const ninjaPath = path.join(buildDir, "build.ninja");
  const cachePath = path.join(buildDir, "CMakeCache.txt");
  if (!fs.existsSync(ninjaPath) || !fs.existsSync(cachePath)) return false;

  const buildDirNorm = path.resolve(buildDir).replace(/\\/g, "/").toLowerCase();
  const installPrefixNorm = path.resolve(buildDir, "..").replace(/\\/g, "/").toLowerCase();
  const cacheText = fs.readFileSync(cachePath, "utf8");
  const sharedLibs = readCmakeCacheValue(cacheText, "BUILD_SHARED_LIBS");

  if (
    isGpuPrebuildEntryWarm(buildDir) &&
    sharedLibs?.toUpperCase() !== "ON" &&
    !fs.existsSync(path.join(buildDir, "..", "bin", "ggml-base.dll"))
  ) {
    const cacheDir = readCmakeCacheValue(cacheText, "CMAKE_CACHEFILE_DIR");
    if (cacheDir && cacheDir.replace(/\\/g, "/").toLowerCase() !== buildDirNorm) {
      return true;
    }
    const installPrefix = readCmakeCacheValue(cacheText, "CMAKE_INSTALL_PREFIX");
    if (
      !installPrefix ||
      installPrefix.replace(/\\/g, "/").toLowerCase() !== installPrefixNorm
    ) {
      return true;
    }
    if (process.platform === "win32") {
      const cudaLauncher = readCmakeCacheValue(cacheText, "CMAKE_CUDA_COMPILER_LAUNCHER");
      if (cudaLauncher?.trim()) {
        return true;
      }
    }
    const ninjaText = fs.readFileSync(ninjaPath, "utf8").toLowerCase();
    if (ninjaText.includes("gpu-prebuild")) {
      return true;
    }
    return false;
  }

  const ninjaText = fs.readFileSync(ninjaPath, "utf8").toLowerCase();
  if (ninjaText.includes("gpu-prebuild")) {
    return true;
  }
  if (ninjaText.includes("/bin/ggml-base.dll") || ninjaText.includes("\\bin\\ggml-base.dll")) {
    return true;
  }
  if (ninjaText.includes("shared_library target ggml-base")) {
    return true;
  }

  if (sharedLibs?.toUpperCase() === "ON") {
    return true;
  }

  const outBin = path.join(buildDir, "..", "bin", "ggml-base.dll");
  if (fs.existsSync(outBin)) {
    return true;
  }

  const cacheDir = readCmakeCacheValue(cacheText, "CMAKE_CACHEFILE_DIR");
  if (cacheDir && cacheDir.replace(/\\/g, "/").toLowerCase() !== buildDirNorm) {
    return true;
  }
  const installPrefix = readCmakeCacheValue(cacheText, "CMAKE_INSTALL_PREFIX");
  if (
    !installPrefix ||
    installPrefix.replace(/\\/g, "/").toLowerCase() !== installPrefixNorm
  ) {
    return true;
  }
  if (process.platform === "win32") {
    const cudaLauncher = readCmakeCacheValue(cacheText, "CMAKE_CUDA_COMPILER_LAUNCHER");
    if (cudaLauncher?.trim()) {
      return true;
    }
  }
  return false;
}

/**
 * Drop `-sys` out/build trees whose CMake cache still pins a CUDA compiler launcher on Windows.
 * Those trees fail Ninja links with MSVC LNK1181 (missing .obj) under parallel nvcc builds.
 * @param {string} jarvisRoot
 * @param {{ profile?: "debug"|"release", logPrefix?: string }} [opts]
 */
export function purgeWindowsStaleCudaCompilerLauncherCaches(jarvisRoot, opts = {}) {
  const profile = opts.profile ?? "debug";
  const logPrefix = opts.logPrefix ?? "gpu-prebuild";
  /** @type {string[]} */
  const purged = [];
  if (process.platform !== "win32") return { purged };

  const buildRoot = path.join(jarvisRoot, "src-tauri", "target", profile, "build");
  if (!fs.existsSync(buildRoot)) return { purged };

  let entries;
  try {
    entries = fs.readdirSync(buildRoot, { withFileTypes: true });
  } catch {
    return { purged };
  }

  for (const { prefix, label } of GPU_NATIVE_BUILD_PREFIXES) {
    for (const ent of entries) {
      if (!ent.isDirectory() || !ent.name.startsWith(prefix)) continue;
      const outBuild = path.join(buildRoot, ent.name, "out", "build");
      const cachePath = path.join(outBuild, "CMakeCache.txt");
      if (!fs.existsSync(cachePath)) continue;
      let cacheText;
      try {
        cacheText = fs.readFileSync(cachePath, "utf8");
      } catch {
        continue;
      }
      const cudaLauncher = readCmakeCacheValue(cacheText, "CMAKE_CUDA_COMPILER_LAUNCHER");
      if (!cudaLauncher?.trim()) continue;
      try {
        fs.rmSync(outBuild, { recursive: true, force: true });
        const rel = path.relative(jarvisRoot, outBuild);
        purged.push(rel);
        console.log(
          `${logPrefix}: removed stale Windows CUDA compiler-launcher CMake tree (${label}, ${rel})`,
        );
      } catch (err) {
        console.warn(
          `${logPrefix}: could not remove stale CUDA launcher tree at ${path.relative(jarvisRoot, outBuild)}:`,
          err instanceof Error ? err.message : err,
        );
      }
    }
  }

  return { purged };
}

/**
 * @param {string} jarvisRoot
 * @param {GpuPrebuildEntryId} entryId
 * @param {string} sysDir
 * @param {string} outBuild
 * @param {NodeJS.ProcessEnv} env
 */
function finalizeSeededOutBuild(jarvisRoot, entryId, sysDir, outBuild, env) {
  const outDir = path.join(sysDir, "out");
  let sourceDir;
  if (entryId === "whisper") {
    sourceDir = path.join(outDir, "whisper.cpp");
    if (!fs.existsSync(sourceDir)) return false;
  } else {
    sourceDir = resolveLlamaCppSourceDir(jarvisRoot);
    if (!sourceDir) return false;
  }

  rewriteSeededCmakeCache(outBuild, { homeDir: sourceDir });
  removeSharedGgmlInstallArtifacts(outDir);

  if (isGpuPrebuildEntryWarm(outBuild) && !seededCmakeBuildNeedsRegen(outBuild)) {
    if (!runCmakeInstallOnly(outBuild, env)) {
      console.warn(
        "gpu-prebuild: cmake install failed after seed; warm ggml-cuda.lib remains in build tree",
      );
    }
    return true;
  }
  if (!regenerateSeededCmakeBuildNinja(outBuild, sourceDir, env, entryId)) return false;
  return runCmakeInstallOnly(outBuild, env);
}

/**
 * @param {string} jarvisRoot
 * @param {object} opts
 * @param {string} [opts.arch]
 * @param {string} [opts.generator]
 * @param {"debug"|"release"} [opts.profile]
 * @param {boolean} [opts.force]
 * @param {{ logPrefix?: string }} [opts.log]
 * @param {NodeJS.ProcessEnv} [opts.env]
 * @returns {{ seeded: string[], skipped: string[], warnings: string[] }}
 */
export function seedGpuPrebuildCache(jarvisRoot, opts = {}) {
  const logPrefix = opts.log?.logPrefix ?? "gpu-prebuild";
  const arch = opts.arch ?? resolveCudaArchitectures();
  const generator = opts.generator ?? resolveCudaCmakeGenerator();
  const profile = opts.profile ?? "debug";
  const force = opts.force === true;
  const env = opts.env ?? process.env;

  /** @type {string[]} */
  const seeded = [];
  /** @type {string[]} */
  const skipped = [];
  /** @type {string[]} */
  const warnings = [];

  if (!isGpuPrebuildCacheWarm(jarvisRoot, arch, generator)) {
    warnings.push(
      `GPU prebuild cache not warm for arch=${arch} generator=${generator} — run npm run prebuild:gpu-cuda`,
    );
    return { seeded, skipped, warnings };
  }

  const cacheRoot = resolveGpuPrebuildCacheRoot(jarvisRoot, arch);
  const buildRoot = path.join(jarvisRoot, "src-tauri", "target", profile, "build");
  if (!fs.existsSync(buildRoot)) {
    skipped.push("cargo target/build missing (first cargo build will create -sys dirs)");
    return { seeded, skipped, warnings };
  }

  let entries;
  try {
    entries = fs.readdirSync(buildRoot, { withFileTypes: true });
  } catch {
    warnings.push(`could not read ${path.relative(jarvisRoot, buildRoot)}`);
    return { seeded, skipped, warnings };
  }

  const cacheByEntry = {
    whisper: resolveGpuPrebuildEntryDirs("whisper", cacheRoot).buildDir,
    llama: resolveGpuPrebuildEntryDirs("llama", cacheRoot).buildDir,
  };

  /** @type {Record<string, GpuPrebuildEntryId>} */
  const prefixToEntry = {
    "whisper-rs-sys-": "whisper",
    "llama-cpp-sys-2-": "llama",
  };

  for (const { prefix, label } of GPU_NATIVE_BUILD_PREFIXES) {
    const entryId = prefixToEntry[prefix];
    const matches = entries.filter((d) => d.isDirectory() && d.name.startsWith(prefix));
    if (matches.length === 0) {
      skipped.push(`${label}: no ${prefix}* dir in target yet`);
      continue;
    }

    const cacheBuild = cacheByEntry[entryId];
    if (!cacheBuild || !isGpuPrebuildEntryWarm(cacheBuild)) {
      skipped.push(`${label}: prebuild entry missing`);
      continue;
    }

    for (const ent of matches) {
      const sysDir = path.join(buildRoot, ent.name);
      const outBuild = path.join(sysDir, "out", "build");
      const relOut = path.relative(jarvisRoot, outBuild);

      if (
        !force &&
        isGpuPrebuildEntryWarm(outBuild) &&
        !seededCmakeBuildNeedsRegen(outBuild)
      ) {
        skipped.push(`${label} (${relOut}): out/build already has ggml-cuda`);
        continue;
      }

      if (entryId === "whisper") {
        const whisperCpp = path.join(sysDir, "out", "whisper.cpp");
        if (!fs.existsSync(whisperCpp)) {
          skipped.push(
            `${label} (${relOut}): out/whisper.cpp missing — cargo must copy sources once before seeding whisper`,
          );
          continue;
        }
      }

      try {
        copyDirRecursive(cacheBuild, outBuild);
        const cacheEntry = resolveGpuPrebuildEntryDirs(entryId, cacheRoot);
        const homeDir =
          entryId === "whisper"
            ? path.join(sysDir, "out", "whisper.cpp")
            : resolveLlamaCppSourceDir(jarvisRoot);
        /** @type {Array<[string, string]>} */
        const rewritePairs = [
          [cacheBuild, outBuild],
          [cacheEntry.sourceDir, homeDir ?? ""],
        ];
        const cacheCMake = path.join(cacheBuild, "CMakeCache.txt");
        if (fs.existsSync(cacheCMake)) {
          const staleInstallPrefix = readCmakeCacheValue(
            fs.readFileSync(cacheCMake, "utf8"),
            "CMAKE_INSTALL_PREFIX",
          );
          if (staleInstallPrefix?.trim()) {
            rewritePairs.push([staleInstallPrefix, path.resolve(outBuild, "..")]);
          }
        }
        if (homeDir) {
          rewritePathPrefixesInSeededBuildTree(outBuild, rewritePairs);
        }
        finalizeSeededOutBuild(jarvisRoot, entryId, sysDir, outBuild, env);
        seeded.push(relOut);
        console.log(`${logPrefix}: seeded ${label} CMake tree from prebuild cache (${relOut})`);
      } catch (err) {
        warnings.push(
          `${label} (${relOut}): seed failed (${err instanceof Error ? err.message : err})`,
        );
      }
    }
  }

  return { seeded, skipped, warnings };
}

/**
 * @param {string} cacheKey e.g. CMAKE_CUDA_HOST_COMPILER
 * @param {string} value
 */
export function cmakeCacheDefineArg(cacheKey, value) {
  const trimmed = value?.trim();
  if (!trimmed) return null;
  const cmakeValue = trimmed.replace(/\\/g, "/");
  return `-D${cacheKey}=${cmakeValue}`;
}

/**
 * @param {object} opts
 * @param {string} opts.sourceDir
 * @param {string} opts.buildDir
 * @param {NodeJS.ProcessEnv} opts.env
 * @param {Record<string, string>} opts.defines
 * @returns {string[]}
 */
export function buildCmakeConfigureArgv({ sourceDir, buildDir, env, defines }) {
  const generator = env.CMAKE_GENERATOR?.trim();
  if (!generator) {
    throw new Error("CMAKE_GENERATOR not set");
  }

  /** @type {string[]} */
  const configureArgs = [
    "-S",
    sourceDir,
    "-B",
    buildDir,
    "-G",
    generator,
    "-DCMAKE_BUILD_TYPE=Release",
  ];

  const cacheEntries = [
    "CMAKE_MAKE_PROGRAM",
    "CMAKE_C_COMPILER",
    "CMAKE_CXX_COMPILER",
    "CMAKE_CUDA_COMPILER",
    "CMAKE_CUDA_HOST_COMPILER",
    "CMAKE_RC_COMPILER",
    "CMAKE_MT",
    "CMAKE_CUDA_ARCHITECTURES",
    "CMAKE_CUDA_FLAGS",
  ];
  for (const key of cacheEntries) {
    const arg = cmakeCacheDefineArg(key, env[key]);
    if (arg) configureArgs.push(arg);
  }
  for (const [key, value] of Object.entries(defines)) {
    configureArgs.push(`-D${key}=${value}`);
  }
  return configureArgs;
}

/**
 * @param {string} label
 * @param {string} sourceDir
 * @param {string} buildDir
 * @param {NodeJS.ProcessEnv} env
 * @param {Record<string, string>} defines
 */
export function runCmakeReleasePrebuild({ label, sourceDir, buildDir, env, defines }) {
  const logPrefix = "prebuild-gpu-cuda";
  if (!fs.existsSync(sourceDir)) {
    throw new Error(`${label}: source dir missing: ${sourceDir}`);
  }

  fs.mkdirSync(buildDir, { recursive: true });

  const generator = env.CMAKE_GENERATOR?.trim();
  if (!generator) {
    throw new Error(`${label}: CMAKE_GENERATOR not set`);
  }

  const parallel = env.CMAKE_BUILD_PARALLEL_LEVEL?.trim() || resolveCudaBuildParallelLevel();
  const configureArgs = buildCmakeConfigureArgv({ sourceDir, buildDir, env, defines });

  const cmake = env.CMAKE?.trim() || "cmake";
  console.log(`${logPrefix}: [${label}] cmake configure (${generator}, Release)`);
  const cfg = spawnSync(cmake, configureArgs, {
    env,
    stdio: "inherit",
    shell: false,
  });
  if (cfg.status !== 0) {
    throw new Error(`${label}: cmake configure failed (exit ${cfg.status ?? 1})`);
  }

  /** @type {string[]} */
  const buildArgs = ["--build", buildDir, "--config", "Release", "--parallel", parallel];
  console.log(`${logPrefix}: [${label}] cmake --build (parallel=${parallel})`);
  const build = spawnSync(cmake, buildArgs, {
    env,
    stdio: "inherit",
    shell: false,
  });
  if (build.status !== 0) {
    throw new Error(`${label}: cmake build failed (exit ${build.status ?? 1})`);
  }

  const artifact = findGgmlCudaLibInBuildDir(buildDir);
  if (!artifact) {
    throw new Error(`${label}: build finished but ggml-cuda lib not found under ${buildDir}`);
  }
  console.log(`${logPrefix}: [${label}] ggml-cuda artifact: ${artifact}`);
}

/**
 * @param {string} fromDir
 * @param {string} toDir
 */
export function syncSourceTreeForPrebuild(fromDir, toDir) {
  fs.mkdirSync(path.dirname(toDir), { recursive: true });
  if (fs.existsSync(toDir)) {
    fs.rmSync(toDir, { recursive: true, force: true });
  }
  fs.cpSync(fromDir, toDir, { recursive: true, force: true });
}

/** Whisper-rs-sys CUDA CMake defines (Release profile). */
export function whisperCudaPrebuildDefines() {
  return {
    BUILD_SHARED_LIBS: "OFF",
    WHISPER_ALL_WARNINGS: "OFF",
    WHISPER_ALL_WARNINGS_3RD_PARTY: "OFF",
    WHISPER_BUILD_TESTS: "OFF",
    WHISPER_BUILD_EXAMPLES: "OFF",
    GGML_CUDA: "ON",
    GGML_METAL: "OFF",
    GGML_OPENMP: "OFF",
    WHISPER_CCACHE: "OFF",
  };
}

/** llama-cpp-sys-2 CUDA CMake defines (Release profile). */
export function llamaCudaPrebuildDefines() {
  return {
    BUILD_SHARED_LIBS: "OFF",
    LLAMA_BUILD_TESTS: "OFF",
    LLAMA_BUILD_EXAMPLES: "OFF",
    LLAMA_BUILD_SERVER: "OFF",
    LLAMA_BUILD_TOOLS: "OFF",
    LLAMA_BUILD_COMMON: "ON",
    LLAMA_CURL: "OFF",
    GGML_CUDA: "ON",
    GGML_OPENMP: "OFF",
    GGML_LLAMAFILE: "OFF",
    GGML_CCACHE: "OFF",
  };
}

function stripCompilerCacheFromPath(envObj) {
  const pathKey = process.platform === "win32" ? "Path" : "PATH";
  const prev = envObj[pathKey] ?? envObj.PATH ?? "";
  const filtered = prev.split(path.delimiter).filter((segment) => {
    if (!segment.trim()) return false;
    if (fs.existsSync(path.join(segment, "ccache.exe"))) return false;
    if (fs.existsSync(path.join(segment, "sccache.exe"))) return false;
    return true;
  });
  envObj[pathKey] = filtered.join(path.delimiter);
  if (pathKey === "Path") envObj.PATH = envObj.Path;
}

/**
 * @param {NodeJS.ProcessEnv} baseEnv
 * @param {{ logPrefix?: string, skipCompilerCache?: boolean }} [opts]
 */
export function buildCudaPrebuildProcessEnv(baseEnv = process.env, opts = {}) {
  const logPrefix = opts.logPrefix ?? "prebuild-gpu-cuda";
  /** @type {NodeJS.ProcessEnv} */
  const env = { ...baseEnv };
  applyCudaCmakeGenerator(env, { logPrefix });
  applyCudaBuildTuningEnv(env);
  if (opts.skipCompilerCache === true) {
    delete env.CMAKE_C_COMPILER_LAUNCHER;
    delete env.CMAKE_CXX_COMPILER_LAUNCHER;
    delete env.CMAKE_CUDA_COMPILER_LAUNCHER;
    stripCompilerCacheFromPath(env);
  } else {
    applyCompilerCacheLauncherEnv(env, { logPrefix, warnIfMissing: false });
  }
  return env;
}

/**
 * @param {string} jarvisRoot
 * @param {string} [arch]
 */
export function summarizeGpuPrebuildCache(jarvisRoot, arch = resolveCudaArchitectures()) {
  const cacheRoot = resolveGpuPrebuildCacheRoot(jarvisRoot, arch);
  const manifest = readGpuPrebuildManifest(cacheRoot);
  const whisper = resolveGpuPrebuildEntryDirs("whisper", cacheRoot);
  const llama = resolveGpuPrebuildEntryDirs("llama", cacheRoot);

  return {
    arch,
    cacheRoot,
    manifest,
    warm: isGpuPrebuildCacheWarm(jarvisRoot, arch, manifest?.generator),
    entries: {
      whisper: {
        buildDir: whisper.buildDir,
        warm: isGpuPrebuildEntryWarm(whisper.buildDir),
        artifact: findGgmlCudaLibInBuildDir(whisper.buildDir),
      },
      llama: {
        buildDir: llama.buildDir,
        warm: isGpuPrebuildEntryWarm(llama.buildDir),
        artifact: findGgmlCudaLibInBuildDir(llama.buildDir),
      },
    },
  };
}
