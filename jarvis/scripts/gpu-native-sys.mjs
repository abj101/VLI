/**
 * GPU native `-sys` crate CMake filesystem seam — cache layout, validation, purge.
 */

import fs from "fs";
import path from "path";

/** Cargo `-sys` trees that compile ggml CUDA during a GPU dev build. */
export const GPU_NATIVE_BUILD_PREFIXES = [
  { prefix: "whisper-rs-sys-", label: "whisper-rs-sys" },
  { prefix: "llama-cpp-sys-2-", label: "llama-cpp-sys" },
];

/**
 * Score a cargo `-sys` out/build tree for progress/diagnostics when multiple hashed dirs exist.
 * @param {string} outBuild
 * @returns {{ score: number, mtime: number }}
 */
export function scoreGpuSysOutBuildDir(outBuild) {
  let score = 0;
  let mtime = 0;
  if (!fs.existsSync(outBuild)) {
    return { score, mtime };
  }

  const ggmlLib = path.join(outBuild, "ggml", "src", "ggml-cuda", "ggml-cuda.lib");
  const altLib = path.join(outBuild, "lib", "ggml-cuda.lib");
  if (fs.existsSync(ggmlLib) || fs.existsSync(altLib)) score += 1000;
  if (fs.existsSync(path.join(outBuild, "CMakeCache.txt"))) score += 100;
  if (fs.existsSync(path.join(outBuild, "build.ninja"))) score += 50;
  if (fs.existsSync(path.join(outBuild, "Makefile"))) score += 50;

  try {
    mtime = fs.statSync(outBuild).mtimeMs;
  } catch {
    mtime = 0;
  }

  return { score, mtime };
}

/**
 * Cargo may leave multiple `whisper-rs-sys-<hash>` dirs; pick the best out/build for monitoring.
 * @param {string} buildRoot target/{profile}/build
 * @param {string} prefix e.g. whisper-rs-sys-
 * @returns {string | null}
 */
export function resolveBestGpuSysOutBuildDir(buildRoot, prefix) {
  if (!fs.existsSync(buildRoot)) return null;

  let entries;
  try {
    entries = fs.readdirSync(buildRoot, { withFileTypes: true });
  } catch {
    return null;
  }

  const matches = entries.filter((d) => d.isDirectory() && d.name.startsWith(prefix));
  if (matches.length === 0) return null;
  if (matches.length === 1) {
    return path.join(buildRoot, matches[0].name, "out", "build");
  }

  /** @type {{ outBuild: string, score: number, mtime: number }[]} */
  const candidates = [];
  for (const ent of matches) {
    const outBuild = path.join(buildRoot, ent.name, "out", "build");
    const { score, mtime } = scoreGpuSysOutBuildDir(outBuild);
    if (score > 0 || fs.existsSync(outBuild)) {
      candidates.push({ outBuild, score, mtime });
    }
  }

  if (candidates.length === 0) {
    return path.join(buildRoot, matches[0].name, "out", "build");
  }

  candidates.sort((a, b) => b.score - a.score || b.mtime - a.mtime);
  return candidates[0].outBuild;
}

/**
 * @param {string} buildRoot
 * @param {string} prefix
 * @returns {string | null} cargo `-sys` build dir (parent of out/)
 */
export function resolveBestGpuSysDir(buildRoot, prefix) {
  const outBuild = resolveBestGpuSysOutBuildDir(buildRoot, prefix);
  if (!outBuild) return null;
  return path.dirname(path.dirname(outBuild));
}

/**
 * @param {string} cacheText
 * @param {string} key
 */
export function readCmakeCacheValue(cacheText, key) {
  const patterns = [
    new RegExp(`^${key}:INTERNAL=(.+)$`, "m"),
    new RegExp(`^${key}:PATH=(.+)$`, "m"),
    new RegExp(`^${key}:STRING=(.+)$`, "m"),
    new RegExp(`^${key}:BOOL=(.+)$`, "m"),
    new RegExp(`^${key}:FILEPATH=(.+)$`, "m"),
  ];
  for (const re of patterns) {
    const match = cacheText.match(re);
    if (match?.[1]?.trim()) return match[1].trim();
  }
  return null;
}

/**
 * @param {string} cacheText
 * @returns {string | null}
 */
export function readCachedCmakeGenerator(cacheText) {
  const internal = cacheText.match(/^CMAKE_GENERATOR:INTERNAL=(.+)$/m);
  if (internal?.[1]?.trim()) return internal[1].trim();
  const uninit = cacheText.match(/^CMAKE_GENERATOR:UNINITIALIZED=(.+)$/m);
  if (uninit?.[1]?.trim()) return uninit[1].trim();
  const plain = cacheText.match(/^CMAKE_GENERATOR:STRING=(.+)$/m);
  if (plain?.[1]?.trim()) return plain[1].trim();
  if (/CMAKE_MAKE_PROGRAM:.*ninja/i.test(cacheText)) return "Ninja";
  if (/CMAKE_CUDA_COMPILER:/m.test(cacheText)) return "NMake Makefiles";
  return null;
}

/** CMake cache data lines use KEY:TYPE=VALUE — bare KEY=VALUE breaks cmake --build. */
const CMAKE_CACHE_LINE = /^[^:]+:[A-Z][A-Z0-9_]*=/;

/** @type {ReadonlySet<string>} */
const CMAKE_CACHE_BOOL_KEYS = new Set([
  "BUILD_SHARED_LIBS",
  "CMAKE_CUDA_COMPILER_WORKS",
  "CMAKE_CXX_COMPILER_WORKS",
  "CMAKE_C_COMPILER_WORKS",
  "GGML_CUDA",
]);

/**
 * Repair bare KEY=VALUE lines (legacy path rewrite on CMakeCache.txt stripped TYPE).
 * @param {string} cacheText
 */
export function repairCmakeCacheMalformedEntries(cacheText) {
  return cacheText
    .split(/\r?\n/)
    .map((line) => {
      const trimmed = line.trim();
      if (!trimmed || trimmed.startsWith("#") || trimmed.startsWith("//")) return line;
      if (CMAKE_CACHE_LINE.test(trimmed)) return line;

      const bare = trimmed.match(/^([A-Z][A-Z0-9_]+)=(.*)$/);
      if (!bare) return line;

      const key = bare[1];
      const value = bare[2];
      const type =
        CMAKE_CACHE_BOOL_KEYS.has(key) ||
        /^(ON|OFF|TRUE|FALSE|YES|NO|0|1)$/i.test(value)
          ? "BOOL"
          : "STRING";
      return `${key}:${type}=${value}`;
    })
    .join("\n");
}

/**
 * @param {string} cacheText
 */
export function isCmakeCacheWellFormed(cacheText) {
  for (const line of cacheText.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#") || trimmed.startsWith("//")) continue;
    if (!CMAKE_CACHE_LINE.test(trimmed)) return false;
    if (trimmed.indexOf("=") < 0) return false;
  }
  return true;
}

/**
 * True when CMake wrote CMakeCache.txt but never finished configure (no generator build file).
 * @param {string} outBuild
 * @param {string | null} cachedGenerator
 */
export function isCmakeBuildTreeIncomplete(outBuild, cachedGenerator) {
  const cachePath = path.join(outBuild, "CMakeCache.txt");
  if (!fs.existsSync(cachePath)) return false;

  const gen = cachedGenerator?.trim();
  if (gen === "Ninja") {
    return !fs.existsSync(path.join(outBuild, "build.ninja"));
  }
  if (gen === "NMake Makefiles") {
    return !fs.existsSync(path.join(outBuild, "Makefile"));
  }
  return false;
}

/**
 * @param {string} jarvisRoot
 * @param {"debug"|"release"} [profile]
 * @returns {{ cleared: string[] }}
 */
export function clearUnusableGpuSysCmakeCaches(jarvisRoot, profile = "debug") {
  /** @type {string[]} */
  const cleared = [];

  const buildRoot = path.join(jarvisRoot, "src-tauri", "target", profile, "build");
  if (!fs.existsSync(buildRoot)) return { cleared };

  let entries;
  try {
    entries = fs.readdirSync(buildRoot, { withFileTypes: true });
  } catch {
    return { cleared };
  }

  for (const { prefix, label } of GPU_NATIVE_BUILD_PREFIXES) {
    for (const ent of entries) {
      if (!ent.isDirectory() || !ent.name.startsWith(prefix)) continue;
      const outBuild = path.join(buildRoot, ent.name, "out", "build");
      const cachePath = path.join(outBuild, "CMakeCache.txt");
      if (!fs.existsSync(cachePath)) continue;

      let text;
      try {
        text = fs.readFileSync(cachePath, "utf8");
      } catch {
        continue;
      }

      const cached = readCachedCmakeGenerator(text);
      const unusable =
        !isCmakeCacheWellFormed(text) ||
        isCmakeBuildTreeIncomplete(outBuild, cached);

      if (!unusable) continue;

      try {
        fs.rmSync(outBuild, { recursive: true, force: true });
        cleared.push(path.relative(jarvisRoot, outBuild));
      } catch (err) {
        console.warn(
          `gpu-native-sys: could not clear unusable ${label} CMake cache at ${path.relative(jarvisRoot, outBuild)}:`,
          err instanceof Error ? err.message : err,
        );
      }
    }
  }

  if (cleared.length > 0) {
    console.warn(
      `gpu-native-sys: cleared unusable GPU -sys CMake cache (${cleared.length} dir(s)) — corrupt or interrupted configure.`,
    );
  }

  return { cleared };
}
