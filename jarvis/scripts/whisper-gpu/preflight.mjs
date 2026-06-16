/**
 * Pre-build checks: Cargo lock contention, first CUDA build notice.
 */

import fs from "fs";
import path from "path";
import { spawnSync } from "child_process";

import {
  GPU_NATIVE_BUILD_PREFIXES,
  clearUnusableGpuSysCmakeCaches,
  isCmakeBuildTreeIncomplete,
  readCachedCmakeGenerator,
  resolveBestGpuSysDir,
} from "../gpu-native-sys.mjs";

export { GPU_NATIVE_BUILD_PREFIXES };

/** @returns {string[]} */
export function cargoLockPaths(jarvisRoot) {
  return [
    path.join(jarvisRoot, "src-tauri", "target", ".cargo-lock"),
    path.join(jarvisRoot, "src-tauri", "target", "debug", ".cargo-lock"),
    path.join(jarvisRoot, "src-tauri", "target", "release", ".cargo-lock"),
  ];
}

/**
 * True when a cargo/rustc process is likely building this crate's target dir.
 * @param {string} jarvisRoot
 */
export function isActiveCargoBuildForJarvis(jarvisRoot) {
  const targetMarker = path
    .join(jarvisRoot, "src-tauri")
    .replace(/\//g, "\\")
    .toLowerCase();
  const markerAlt = targetMarker.replace(/\\/g, "/");

  if (process.platform === "win32") {
    const ps = spawnSync(
      "powershell.exe",
      [
        "-NoProfile",
        "-Command",
        [
          "Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |",
          "Where-Object { $_.Name -in @('cargo.exe','rustc.exe') } |",
          "ForEach-Object { $_.CommandLine }",
        ].join(" "),
      ],
      { encoding: "utf8", windowsHide: true },
    );
    const lines = `${ps.stdout ?? ""}\n${ps.stderr ?? ""}`.split(/\r?\n/);
    for (const line of lines) {
      const lower = line.toLowerCase();
      if (!lower.includes("cargo") && !lower.includes("rustc")) continue;
      if (lower.includes(targetMarker) || lower.includes(markerAlt)) {
        return true;
      }
    }
    return false;
  }

  const pgrep = spawnSync("pgrep", ["-af", "cargo"], { encoding: "utf8" });
  if (pgrep.status === 0) {
    const text = pgrep.stdout ?? "";
    if (text.includes("src-tauri") && text.toLowerCase().includes(path.basename(jarvisRoot).toLowerCase())) {
      return true;
    }
  }
  return false;
}

/**
 * @param {string} lockPath
 * @returns {{ removed: boolean, inUse: boolean }}
 */
function tryClearStaleCargoLock(lockPath) {
  if (!fs.existsSync(lockPath)) {
    return { removed: false, inUse: false };
  }
  try {
    fs.unlinkSync(lockPath);
    return { removed: true, inUse: false };
  } catch (err) {
    const code = /** @type {NodeJS.ErrnoException} */ (err).code;
    if (code === "EBUSY" || code === "EPERM" || code === "EACCES") {
      return { removed: false, inUse: true };
    }
    throw err;
  }
}

/**
 * Clears leftover `.cargo-lock` files from interrupted builds. Blocks only when a lock
 * is held by a live cargo/rustc process for this project.
 * @param {string} jarvisRoot
 * @returns {{ blocked: boolean, message?: string, cleared?: string[] }}
 */
export function checkCargoBuildLock(jarvisRoot) {
  if (process.env.WHISPER_IGNORE_CARGO_LOCK === "1") {
    return { blocked: false };
  }

  const existing = cargoLockPaths(jarvisRoot).filter((p) => fs.existsSync(p));
  if (existing.length === 0) {
    return { blocked: false };
  }

  /** @type {string[]} */
  const cleared = [];
  let lockHeld = false;

  for (const lockPath of existing) {
    const result = tryClearStaleCargoLock(lockPath);
    if (result.removed) {
      cleared.push(path.relative(jarvisRoot, lockPath));
      continue;
    }
    if (result.inUse) {
      lockHeld = true;
    }
  }

  if (cleared.length > 0) {
    console.warn(
      `whisper-gpu: removed stale Cargo lock file(s): ${cleared.join(", ")} (interrupted prior build).`,
    );
  }

  const stillPresent = cargoLockPaths(jarvisRoot).filter((p) => fs.existsSync(p));
  if (stillPresent.length === 0) {
    return { blocked: false, cleared };
  }

  if (lockHeld || isActiveCargoBuildForJarvis(jarvisRoot)) {
    const rel = stillPresent.map((p) => path.relative(jarvisRoot, p)).join(", ");
    return {
      blocked: true,
      message: `Another Cargo build is using src-tauri/target/ (${rel}). Wait for it to finish, stop that process, or set WHISPER_IGNORE_CARGO_LOCK=1 to force (risky).`,
      cleared,
    };
  }

  // Lock files reappeared or could not be deleted without EBUSY — try once more, then warn-only.
  for (const lockPath of stillPresent) {
    tryClearStaleCargoLock(lockPath);
  }
  const afterRetry = cargoLockPaths(jarvisRoot).filter((p) => fs.existsSync(p));
  if (afterRetry.length === 0) {
    return { blocked: false, cleared };
  }

  console.warn(
    `whisper-gpu: could not remove ${afterRetry.map((p) => path.relative(jarvisRoot, p)).join(", ")}; continuing — Cargo will block or wait if the lock is live.`,
  );
  return { blocked: false, cleared };
}

/**
 * @param {string} jarvisRoot
 * @param {"debug"|"release"} [profile]
 */
export function isLikelyFirstCudaWhisperBuild(jarvisRoot, profile = "debug") {
  const buildRoot = path.join(jarvisRoot, "src-tauri", "target", profile, "build");
  if (!fs.existsSync(buildRoot)) return true;

  let entries;
  try {
    entries = fs.readdirSync(buildRoot, { withFileTypes: true });
  } catch {
    return true;
  }

  const sysDirs = entries
    .filter((d) => d.isDirectory() && d.name.startsWith("whisper-rs-sys-"))
    .map((d) => path.join(buildRoot, d.name));

  if (sysDirs.length === 0) return true;

  for (const sysDir of sysDirs) {
    const markers = [
      path.join(sysDir, "out", "build", "ggml", "src", "ggml-cuda", "libggml-cuda.a"),
      path.join(sysDir, "out", "build", "ggml", "src", "ggml-cuda", "ggml-cuda.lib"),
      path.join(sysDir, "out", "build", "lib", "ggml-cuda.lib"),
    ];
    if (markers.some((m) => fs.existsSync(m))) {
      return false;
    }
    const outBuild = path.join(sysDir, "out", "build");
    if (fs.existsSync(outBuild)) {
      try {
        const walk = (dir, depth) => {
          if (depth > 6) return false;
          for (const ent of fs.readdirSync(dir, { withFileTypes: true })) {
            const p = path.join(dir, ent.name);
            if (ent.isFile() && /ggml-cuda/i.test(ent.name) && /\.(lib|a)$/i.test(ent.name)) {
              return true;
            }
            if (ent.isDirectory() && walk(p, depth + 1)) return true;
          }
          return false;
        };
        if (walk(outBuild, 0)) return false;
      } catch {
        /* ignore */
      }
    }
  }
  return true;
}

/**
 * Remove stale GPU `-sys` CMake trees when generator changes (VS ↔ NMake/Ninja hangs or errors).
 * @param {string} jarvisRoot
 * @param {string} intendedGenerator e.g. "NMake Makefiles" or "Visual Studio 17 2022"
 * @param {"debug"|"release"} [profile]
 * @returns {{ cleared: string[] }}
 */
export function clearGpuSysCmakeCacheOnGeneratorMismatch(
  jarvisRoot,
  intendedGenerator,
  profile = "debug",
) {
  /** @type {string[]} */
  const cleared = [];
  if (!intendedGenerator?.trim()) return { cleared };

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
      if (!cached || cached === intendedGenerator) continue;

      try {
        fs.rmSync(outBuild, { recursive: true, force: true });
        cleared.push(path.relative(jarvisRoot, outBuild));
      } catch (err) {
        console.warn(
          `whisper-gpu: could not clear mismatched ${label} CMake cache at ${path.relative(jarvisRoot, outBuild)}:`,
          err instanceof Error ? err.message : err,
        );
      }
    }
  }

  if (cleared.length > 0) {
    console.warn(
      `whisper-gpu: cleared GPU -sys CMake cache (${cleared.length} dir(s)) — wrong generator for "${intendedGenerator}".`,
    );
  }
  return { cleared };
}

/**
 * Remove interrupted GPU `-sys` CMake trees (CMakeCache without build.ninja / Makefile).
 * @param {string} jarvisRoot
 * @param {"debug"|"release"} [profile]
 * @returns {{ cleared: string[] }}
 */
export function clearIncompleteGpuSysCmakeCache(jarvisRoot, profile = "debug") {
  return clearUnusableGpuSysCmakeCaches(jarvisRoot, profile);
}

/** @deprecated Use clearGpuSysCmakeCacheOnGeneratorMismatch */
export function clearWhisperRsSysBuildCacheOnGeneratorMismatch(
  jarvisRoot,
  intendedGenerator,
  profile = "debug",
) {
  return clearGpuSysCmakeCacheOnGeneratorMismatch(jarvisRoot, intendedGenerator, profile);
}

/**
 * @param {string} sysDir
 * @returns {string | null}
 */
function findGgmlCudaArtifactInSysDir(sysDir) {
  const markers = [
    path.join(sysDir, "out", "build", "ggml", "src", "ggml-cuda", "libggml-cuda.a"),
    path.join(sysDir, "out", "build", "ggml", "src", "ggml-cuda", "ggml-cuda.lib"),
    path.join(sysDir, "out", "build", "lib", "ggml-cuda.lib"),
  ];
  for (const marker of markers) {
    if (fs.existsSync(marker)) return marker;
  }
  const outBuild = path.join(sysDir, "out", "build");
  if (!fs.existsSync(outBuild)) return null;
  try {
    const walk = (dir, depth) => {
      if (depth > 6) return null;
      for (const ent of fs.readdirSync(dir, { withFileTypes: true })) {
        const p = path.join(dir, ent.name);
        if (ent.isFile() && /ggml-cuda/i.test(ent.name) && /\.(lib|a)$/i.test(ent.name)) {
          return p;
        }
        if (ent.isDirectory()) {
          const nested = walk(p, depth + 1);
          if (nested) return nested;
        }
      }
      return null;
    };
    return walk(outBuild, 0);
  } catch {
    return null;
  }
}

/**
 * @param {string} jarvisRoot
 * @param {"debug"|"release"} [profile]
 * @returns {Array<{ label: string, artifactPath: string | null, sysDir: string | null }>}
 */
export function summarizeGgmlCudaArtifacts(jarvisRoot, profile = "debug") {
  const buildRoot = path.join(jarvisRoot, "src-tauri", "target", profile, "build");
  if (!fs.existsSync(buildRoot)) {
    return GPU_NATIVE_BUILD_PREFIXES.map(({ label }) => ({
      label,
      artifactPath: null,
      sysDir: null,
    }));
  }

  let entries;
  try {
    entries = fs.readdirSync(buildRoot, { withFileTypes: true });
  } catch {
    return GPU_NATIVE_BUILD_PREFIXES.map(({ label }) => ({
      label,
      artifactPath: null,
      sysDir: null,
    }));
  }

  return GPU_NATIVE_BUILD_PREFIXES.map(({ prefix, label }) => {
    const sysDir = resolveBestGpuSysDir(buildRoot, prefix);
    if (!sysDir) {
      return { label, artifactPath: null, sysDir: null };
    }
    return {
      label,
      artifactPath: findGgmlCudaArtifactInSysDir(sysDir),
      sysDir,
    };
  });
}

/**
 * @param {string} jarvisRoot
 * @param {string} intendedGenerator
 * @param {"debug"|"release"} [profile]
 * @returns {Array<{ label: string, cached: string, cachePath: string }>}
 */
export function listGpuSysCmakeGeneratorMismatches(
  jarvisRoot,
  intendedGenerator,
  profile = "debug",
) {
  /** @type {Array<{ label: string, cached: string, cachePath: string }>} */
  const mismatches = [];
  if (!intendedGenerator?.trim()) return mismatches;

  const buildRoot = path.join(jarvisRoot, "src-tauri", "target", profile, "build");
  if (!fs.existsSync(buildRoot)) return mismatches;

  let entries;
  try {
    entries = fs.readdirSync(buildRoot, { withFileTypes: true });
  } catch {
    return mismatches;
  }

  for (const { prefix, label } of GPU_NATIVE_BUILD_PREFIXES) {
    for (const ent of entries) {
      if (!ent.isDirectory() || !ent.name.startsWith(prefix)) continue;
      const cachePath = path.join(buildRoot, ent.name, "out", "build", "CMakeCache.txt");
      if (!fs.existsSync(cachePath)) continue;
      let text;
      try {
        text = fs.readFileSync(cachePath, "utf8");
      } catch {
        continue;
      }
      const cached = readCachedCmakeGenerator(text);
      if (cached && cached !== intendedGenerator) {
        mismatches.push({ label, cached, cachePath });
      }
    }
  }
  return mismatches;
}

/**
 * @param {string} dir
 * @param {number} depth
 * @returns {{ artifactCount: number, newestMtime: number }}
 */
function walkNativeBuildArtifacts(dir, depth) {
  let artifactCount = 0;
  let newestMtime = 0;
  if (depth > 8) return { artifactCount, newestMtime };

  let entries;
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true });
  } catch {
    return { artifactCount, newestMtime };
  }

  for (const ent of entries) {
    const p = path.join(dir, ent.name);
    if (ent.isDirectory()) {
      const nested = walkNativeBuildArtifacts(p, depth + 1);
      artifactCount += nested.artifactCount;
      if (nested.newestMtime > newestMtime) newestMtime = nested.newestMtime;
      continue;
    }
    if (!/\.(obj|o|lib|a|dll|exe|pdb)$/i.test(ent.name)) continue;
    artifactCount += 1;
    try {
      const mtime = fs.statSync(p).mtimeMs;
      if (mtime > newestMtime) newestMtime = mtime;
    } catch {
      /* ignore */
    }
  }
  return { artifactCount, newestMtime };
}

/**
 * @param {string} jarvisRoot
 * @param {"debug"|"release"} [profile]
 * @returns {{
 *   artifactCount: number,
 *   newestAgeSec: number | null,
 *   crates: Array<{ label: string, artifactCount: number, newestAgeSec: number | null }>,
 * }}
 */
export function summarizeGpuNativeBuildActivity(jarvisRoot, profile = "debug") {
  const buildRoot = path.join(jarvisRoot, "src-tauri", "target", profile, "build");
  if (!fs.existsSync(buildRoot)) {
    return { artifactCount: 0, newestAgeSec: null, crates: [] };
  }

  let entries;
  try {
    entries = fs.readdirSync(buildRoot, { withFileTypes: true });
  } catch {
    return { artifactCount: 0, newestAgeSec: null, crates: [] };
  }

  /** @type {Map<string, { label: string, artifactCount: number, newestMtime: number }>} */
  const byLabel = new Map();

  for (const { prefix, label } of GPU_NATIVE_BUILD_PREFIXES) {
    for (const ent of entries) {
      if (!ent.isDirectory() || !ent.name.startsWith(prefix)) continue;
      const walked = walkNativeBuildArtifacts(
        path.join(buildRoot, ent.name, "out", "build"),
        0,
      );
      const slot = byLabel.get(label) ?? {
        label,
        artifactCount: 0,
        newestMtime: 0,
      };
      slot.artifactCount += walked.artifactCount;
      if (walked.newestMtime > slot.newestMtime) slot.newestMtime = walked.newestMtime;
      byLabel.set(label, slot);
    }
  }

  const crates = [...byLabel.values()].map((slot) => ({
    label: slot.label,
    artifactCount: slot.artifactCount,
    newestAgeSec:
      slot.newestMtime > 0
        ? Math.max(0, Math.floor((Date.now() - slot.newestMtime) / 1000))
        : null,
  }));

  let artifactCount = 0;
  let newestMtime = 0;
  for (const crate of crates) {
    artifactCount += crate.artifactCount;
    if (crate.newestAgeSec != null) {
      const mtime = Date.now() - crate.newestAgeSec * 1000;
      if (mtime > newestMtime) newestMtime = mtime;
    }
  }

  const newestAgeSec =
    newestMtime > 0 ? Math.max(0, Math.floor((Date.now() - newestMtime) / 1000)) : null;
  return { artifactCount, newestAgeSec, crates };
}

/**
 * @param {string} jarvisRoot
 * @param {"debug"|"release"} [profile]
 * @returns {{ artifactCount: number, newestAgeSec: number | null }}
 */
export function summarizeWhisperRsSysBuildActivity(jarvisRoot, profile = "debug") {
  const activity = summarizeGpuNativeBuildActivity(jarvisRoot, profile);
  const whisper = activity.crates.find((c) => c.label === "whisper-rs-sys");
  return {
    artifactCount: whisper?.artifactCount ?? 0,
    newestAgeSec: whisper?.newestAgeSec ?? null,
  };
}

/**
 * Warn when a prior GPU `-sys` CMake cache used a different generator (VS vs NMake/Ninja).
 * @param {string} jarvisRoot
 * @param {string} intendedGenerator e.g. "NMake Makefiles" or "Visual Studio 17 2022"
 */
export function warnIfCmakeGeneratorMismatch(jarvisRoot, intendedGenerator) {
  clearIncompleteGpuSysCmakeCache(jarvisRoot);
  const { cleared } = clearGpuSysCmakeCacheOnGeneratorMismatch(jarvisRoot, intendedGenerator);
  if (cleared.length > 0) return;
  if (!intendedGenerator?.trim()) return;

  const mismatches = listGpuSysCmakeGeneratorMismatches(jarvisRoot, intendedGenerator);
  for (const { label, cached } of mismatches) {
    console.warn(
      `whisper-gpu: prior ${label} CMake cache used generator "${cached}"; this run uses "${intendedGenerator}" — expect a full ${label} rebuild.`,
    );
  }
}

export function logFirstCudaBuildNotice(jarvisRoot, profile = "debug") {
  if (!isLikelyFirstCudaWhisperBuild(jarvisRoot, profile)) return;
  console.warn(
    "whisper-gpu: first GPU dev build compiles CUDA for whisper-rs-sys and llama-cpp-sys — often 45–90+ minutes on Windows.",
  );
  console.warn(
    "whisper-gpu: progress may pause near the end (link step). Use one terminal; do not run parallel `cargo` / `tauri dev`.",
  );
}
