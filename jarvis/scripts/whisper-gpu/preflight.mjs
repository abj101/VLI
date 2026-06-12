/**
 * Pre-build checks: Cargo lock contention, first CUDA build notice.
 */

import fs from "fs";
import path from "path";
import { spawnSync } from "child_process";

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
 * @param {string} cacheText
 * @returns {string | null}
 */
function readCachedCmakeGenerator(cacheText) {
  const internal = cacheText.match(/^CMAKE_GENERATOR:INTERNAL=(.+)$/m);
  if (internal?.[1]?.trim()) return internal[1].trim();
  const plain = cacheText.match(/^CMAKE_GENERATOR:STRING=(.+)$/m);
  if (plain?.[1]?.trim()) return plain[1].trim();
  if (/CMAKE_CUDA_COMPILER:/m.test(cacheText)) return "NMake Makefiles";
  if (/CMAKE_GENERATOR:INTERNAL=Ninja/m.test(cacheText)) return "Ninja";
  return null;
}

/**
 * Remove stale whisper-rs-sys CMake trees when generator changes (VS ↔ NMake hangs or errors).
 * @param {string} jarvisRoot
 * @param {string} intendedGenerator e.g. "NMake Makefiles" or "Visual Studio 17 2022"
 * @param {"debug"|"release"} [profile]
 * @returns {{ cleared: string[] }}
 */
export function clearWhisperRsSysBuildCacheOnGeneratorMismatch(
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

  for (const ent of entries) {
    if (!ent.isDirectory() || !ent.name.startsWith("whisper-rs-sys-")) continue;
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
        `whisper-gpu: could not clear mismatched CMake cache at ${path.relative(jarvisRoot, outBuild)}:`,
        err instanceof Error ? err.message : err,
      );
    }
  }

  if (cleared.length > 0) {
    console.warn(
      `whisper-gpu: cleared whisper-rs-sys CMake cache (${cleared.length} dir(s)) — was wrong generator for "${intendedGenerator}".`,
    );
  }
  return { cleared };
}

/**
 * @param {string} jarvisRoot
 * @param {"debug"|"release"} [profile]
 * @returns {{ artifactCount: number, newestAgeSec: number | null }}
 */
export function summarizeWhisperRsSysBuildActivity(jarvisRoot, profile = "debug") {
  const buildRoot = path.join(jarvisRoot, "src-tauri", "target", profile, "build");
  if (!fs.existsSync(buildRoot)) {
    return { artifactCount: 0, newestAgeSec: null };
  }

  let artifactCount = 0;
  let newestMtime = 0;

  /** @param {string} dir @param {number} depth */
  const walk = (dir, depth) => {
    if (depth > 8) return;
    let entries;
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const ent of entries) {
      const p = path.join(dir, ent.name);
      if (ent.isDirectory()) {
        walk(p, depth + 1);
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
  };

  let entries;
  try {
    entries = fs.readdirSync(buildRoot, { withFileTypes: true });
  } catch {
    return { artifactCount: 0, newestAgeSec: null };
  }

  for (const ent of entries) {
    if (!ent.isDirectory() || !ent.name.startsWith("whisper-rs-sys-")) continue;
    walk(path.join(buildRoot, ent.name, "out", "build"), 0);
  }

  const newestAgeSec =
    newestMtime > 0 ? Math.max(0, Math.floor((Date.now() - newestMtime) / 1000)) : null;
  return { artifactCount, newestAgeSec };
}

/**
 * Warn when a prior whisper-rs-sys CMake cache used a different generator (VS vs NMake).
 * @param {string} jarvisRoot
 * @param {string} intendedGenerator e.g. "NMake Makefiles" or "Visual Studio 17 2022"
 */
export function warnIfCmakeGeneratorMismatch(jarvisRoot, intendedGenerator) {
  const { cleared } = clearWhisperRsSysBuildCacheOnGeneratorMismatch(
    jarvisRoot,
    intendedGenerator,
  );
  if (cleared.length > 0) return;
  if (!intendedGenerator?.trim()) return;

  const buildRoot = path.join(jarvisRoot, "src-tauri", "target", "debug", "build");
  if (!fs.existsSync(buildRoot)) return;

  let entries;
  try {
    entries = fs.readdirSync(buildRoot, { withFileTypes: true });
  } catch {
    return;
  }

  for (const ent of entries) {
    if (!ent.isDirectory() || !ent.name.startsWith("whisper-rs-sys-")) continue;
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
      console.warn(
        `whisper-gpu: prior whisper-rs-sys CMake cache used generator "${cached}"; this run uses "${intendedGenerator}" — expect a full whisper-rs-sys rebuild.`,
      );
      return;
    }
  }
}

export function logFirstCudaBuildNotice(jarvisRoot) {
  if (!isLikelyFirstCudaWhisperBuild(jarvisRoot)) return;
  console.warn(
    "whisper-gpu: first whisper-cuda build compiles many CUDA kernels — often 20–45+ minutes on Windows.",
  );
  console.warn(
    "whisper-gpu: progress may pause near the end (link step). Use one terminal; do not run parallel `cargo` / `tauri dev`.",
  );
}
