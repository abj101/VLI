/**
 * GPU native `-sys` build progress — phase detection, ninja_log parsing, stall diagnosis.
 */

import fs from "fs";
import path from "path";

import {
  GPU_NATIVE_BUILD_PREFIXES,
  resolveBestGpuSysOutBuildDir,
} from "./gpu-native-sys.mjs";
import { run } from "./whisper-gpu/detect.mjs";

/** @typedef {"pending"|"configure"|"compile"|"link"|"done"} GpuBuildPhase */

/**
 * @typedef {object} GpuSysCrateBuildSnapshot
 * @property {GpuBuildPhase} phase
 * @property {string} label
 * @property {string | null} outBuildDir
 * @property {number} cuTotal
 * @property {number} cuDone
 * @property {number} objCount
 * @property {number} ninjaDone
 * @property {number} ninjaTotal
 * @property {string | null} lastNinjaTarget
 * @property {boolean} ggmlCudaLibPresent
 * @property {number | null} newestArtifactAgeSec
 * @property {number} cratePercent
 */

/**
 * @typedef {object} GpuNativeBuildProgressSnapshot
 * @property {"debug"|"release"} profile
 * @property {number} elapsedSec
 * @property {number} overallPercent
 * @property {string | null} activeCrate
 * @property {GpuSysCrateBuildSnapshot[]} crates
 * @property {string[]} runningProcesses
 * @property {boolean} prebuildWarm
 */

const GGML_CUDA_LIB_RE = /ggml-cuda/i;
const BUILD_PROCESS_NAMES = [
  "nvcc.exe",
  "ninja.exe",
  "cmake.exe",
  "link.exe",
  "lib.exe",
  "cl.exe",
];

/**
 * @param {string} ninjaLogText
 * @returns {{ done: number, lastTarget: string | null }}
 */
export function parseNinjaLog(ninjaLogText) {
  if (!ninjaLogText?.trim()) {
    return { done: 0, lastTarget: null };
  }

  let done = 0;
  /** @type {string | null} */
  let lastTarget = null;

  for (const line of ninjaLogText.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const parts = trimmed.split(/\s+/);
    if (parts.length < 4) continue;
    done += 1;
    lastTarget = parts.length > 4 ? parts.slice(4).join(" ") : parts[parts.length - 1];
  }

  return { done, lastTarget };
}

/**
 * @param {string} buildNinjaPath
 * @returns {number}
 */
export function countNinjaBuildRules(buildNinjaPath) {
  if (!fs.existsSync(buildNinjaPath)) return 0;
  try {
    const text = fs.readFileSync(buildNinjaPath, "utf8");
    return text.split(/\r?\n/).filter((line) => line.startsWith("build ")).length;
  } catch {
    return 0;
  }
}

/**
 * @param {string} dir
 * @param {number} depth
 * @returns {{ cuTotal: number, cuDone: number, objCount: number, newestMtime: number }}
 */
function walkGgmlCudaCompileTree(dir, depth) {
  let cuTotal = 0;
  let cuDone = 0;
  let objCount = 0;
  let newestMtime = 0;

  if (depth > 10 || !fs.existsSync(dir)) {
    return { cuTotal, cuDone, objCount, newestMtime };
  }

  let entries;
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true });
  } catch {
    return { cuTotal, cuDone, objCount, newestMtime };
  }

  for (const ent of entries) {
    const p = path.join(dir, ent.name);
    if (ent.isDirectory()) {
      const nested = walkGgmlCudaCompileTree(p, depth + 1);
      cuTotal += nested.cuTotal;
      cuDone += nested.cuDone;
      objCount += nested.objCount;
      if (nested.newestMtime > newestMtime) newestMtime = nested.newestMtime;
      continue;
    }
    if (/\.cu$/i.test(ent.name)) {
      cuTotal += 1;
      const objPath = p.replace(/\.cu$/i, ".obj");
      if (fs.existsSync(objPath)) cuDone += 1;
      continue;
    }
    if (/\.obj$/i.test(ent.name)) {
      objCount += 1;
      try {
        const mtime = fs.statSync(p).mtimeMs;
        if (mtime > newestMtime) newestMtime = mtime;
      } catch {
        /* ignore */
      }
    }
  }

  return { cuTotal, cuDone, objCount, newestMtime };
}

/**
 * @param {string} outBuildDir
 * @returns {string | null}
 */
function findGgmlCudaLibInOutBuild(outBuildDir) {
  if (!fs.existsSync(outBuildDir)) return null;
  const markers = [
    path.join(outBuildDir, "ggml", "src", "ggml-cuda", "ggml-cuda.lib"),
    path.join(outBuildDir, "lib", "ggml-cuda.lib"),
  ];
  for (const marker of markers) {
    if (fs.existsSync(marker)) return marker;
  }
  const walk = (dir, depth) => {
    if (depth > 6) return null;
    let entries;
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
      return null;
    }
    for (const ent of entries) {
      const p = path.join(dir, ent.name);
      if (ent.isFile() && GGML_CUDA_LIB_RE.test(ent.name) && /\.(lib|a)$/i.test(ent.name)) {
        return p;
      }
      if (ent.isDirectory()) {
        const nested = walk(p, depth + 1);
        if (nested) return nested;
      }
    }
    return null;
  };
  return walk(outBuildDir, 0);
}

/**
 * @param {string} outBuildDir
 * @param {string} label
 * @param {{ runningProcesses?: string[] }} [opts]
 * @returns {GpuSysCrateBuildSnapshot}
 */
export function inspectGpuSysCrateBuild(outBuildDir, label, opts = {}) {
  const running = opts.runningProcesses ?? [];
  const base = {
    phase: /** @type {GpuBuildPhase} */ ("pending"),
    label,
    outBuildDir: fs.existsSync(outBuildDir) ? outBuildDir : null,
    cuTotal: 0,
    cuDone: 0,
    objCount: 0,
    ninjaDone: 0,
    ninjaTotal: 0,
    lastNinjaTarget: null,
    ggmlCudaLibPresent: false,
    newestArtifactAgeSec: null,
    cratePercent: 0,
  };

  if (!fs.existsSync(outBuildDir)) {
    return base;
  }

  const cachePath = path.join(outBuildDir, "CMakeCache.txt");
  const ninjaPath = path.join(outBuildDir, "build.ninja");
  const makefilePath = path.join(outBuildDir, "Makefile");
  const hasGenerator = fs.existsSync(ninjaPath) || fs.existsSync(makefilePath);
  const ggmlLib = findGgmlCudaLibInOutBuild(outBuildDir);

  if (ggmlLib) {
    base.phase = "done";
    base.ggmlCudaLibPresent = true;
    base.cratePercent = 100;
    return base;
  }

  if (!fs.existsSync(cachePath) && !hasGenerator) {
    return base;
  }

  if (fs.existsSync(cachePath) && !hasGenerator) {
    base.phase = "configure";
    base.cratePercent = 5;
    return base;
  }

  const ninjaLogPath = path.join(outBuildDir, ".ninja_log");
  let ninjaLog = "";
  if (fs.existsSync(ninjaLogPath)) {
    try {
      ninjaLog = fs.readFileSync(ninjaLogPath, "utf8");
    } catch {
      ninjaLog = "";
    }
  }
  const parsed = parseNinjaLog(ninjaLog);
  base.ninjaDone = parsed.done;
  base.lastNinjaTarget = parsed.lastTarget;
  base.ninjaTotal = countNinjaBuildRules(ninjaPath);

  const ggmlCudaDir = path.join(outBuildDir, "ggml", "src", "ggml-cuda");
  const walked = walkGgmlCudaCompileTree(
    fs.existsSync(ggmlCudaDir) ? ggmlCudaDir : outBuildDir,
    0,
  );
  base.cuTotal = walked.cuTotal;
  base.cuDone = walked.cuDone;
  base.objCount = walked.objCount;
  if (walked.newestMtime > 0) {
    base.newestArtifactAgeSec = Math.max(
      0,
      Math.floor((Date.now() - walked.newestMtime) / 1000),
    );
  }

  const linkRunning = running.some((p) => /^(link|lib)\.exe$/i.test(p));
  const staleArtifacts =
    base.newestArtifactAgeSec != null && base.newestArtifactAgeSec >= 120;
  const highObjCount = base.objCount >= 20;

  if (linkRunning || (highObjCount && staleArtifacts && base.ninjaDone > 0)) {
    base.phase = "link";
    base.cratePercent = 92;
    return base;
  }

  base.phase = "compile";
  if (base.ninjaTotal > 0 && base.ninjaDone > 0) {
    base.cratePercent = Math.min(90, Math.round((base.ninjaDone / base.ninjaTotal) * 90));
  } else if (base.cuTotal > 0) {
    base.cratePercent = Math.min(90, Math.round((base.cuDone / base.cuTotal) * 90));
  } else if (base.objCount > 0) {
    base.cratePercent = Math.min(85, 10 + Math.min(75, base.objCount));
  } else {
    base.cratePercent = 8;
  }

  return base;
}

/**
 * @param {GpuSysCrateBuildSnapshot} crate
 * @returns {number}
 */
function crateProgressFraction(crate) {
  if (crate.phase === "done") return 1;
  if (crate.phase === "pending") return 0;
  return Math.max(0, Math.min(1, crate.cratePercent / 100));
}

/**
 * @param {string} jarvisRoot
 * @param {"debug"|"release"} [profile]
 * @param {{ startedAt?: number, runningProcesses?: string[], prebuildWarm?: boolean }} [opts]
 * @returns {GpuNativeBuildProgressSnapshot}
 */
export function summarizeGpuNativeBuildProgress(jarvisRoot, profile = "debug", opts = {}) {
  const startedAt = opts.startedAt ?? Date.now();
  const runningProcesses = opts.runningProcesses ?? probeGpuBuildProcesses();
  const buildRoot = path.join(jarvisRoot, "src-tauri", "target", profile, "build");

  /** @type {GpuSysCrateBuildSnapshot[]} */
  const crates = GPU_NATIVE_BUILD_PREFIXES.map(({ prefix, label }) => {
    const outBuild = resolveBestGpuSysOutBuildDir(buildRoot, prefix);
    return inspectGpuSysCrateBuild(outBuild ?? "", label, { runningProcesses });
  });

  const whisperFrac = crateProgressFraction(crates[0]);
  const llamaFrac = crateProgressFraction(crates[1]);
  const overallPercent = Math.round((whisperFrac * 0.5 + llamaFrac * 0.5) * 100);

  /** @type {string | null} */
  let activeCrate = null;
  for (const crate of crates) {
    if (crate.phase !== "done" && crate.phase !== "pending") {
      activeCrate = crate.label;
      break;
    }
  }
  if (!activeCrate) {
    const pending = crates.find((c) => c.phase === "pending");
    activeCrate = pending?.label ?? crates[crates.length - 1]?.label ?? null;
  }

  return {
    profile,
    elapsedSec: Math.max(0, Math.floor((Date.now() - startedAt) / 1000)),
    overallPercent,
    activeCrate,
    crates,
    runningProcesses,
    prebuildWarm: opts.prebuildWarm ?? false,
  };
}

/**
 * @param {string[]} [names]
 * @returns {string[]}
 */
export function probeGpuBuildProcesses(names = BUILD_PROCESS_NAMES) {
  if (process.platform === "win32") {
    /** @type {string[]} */
    const found = [];
    for (const name of names) {
      const r = run("tasklist", ["/FI", `IMAGENAME eq ${name}`, "/NH"]);
      if (!r.ok) continue;
      if (new RegExp(name.replace(".", "\\."), "i").test(r.stdout)) {
        found.push(name);
      }
    }
    return found;
  }
  /** @type {string[]} */
  const found = [];
  for (const name of names) {
    const base = name.replace(/\.exe$/i, "");
    const r = run("pgrep", ["-x", base]);
    if (r.ok) found.push(name);
  }
  return found;
}

/**
 * @param {GpuNativeBuildProgressSnapshot} snapshot
 * @returns {string}
 */
export function formatGpuBuildProgressBar(snapshot) {
  const pct = Math.max(0, Math.min(100, snapshot.overallPercent));
  const filled = Math.round(pct / 10);
  const bar = `${"█".repeat(filled)}${"░".repeat(10 - filled)}`;
  const active = snapshot.crates.find((c) => c.label === snapshot.activeCrate);
  const phase = active?.phase ?? "pending";
  const label = snapshot.activeCrate ?? "gpu-native";
  let detail = phase;
  if (active) {
    if (phase === "compile" && active.ninjaTotal > 0) {
      detail = `compile ${active.ninjaDone}/${active.ninjaTotal} ninja`;
    } else if (phase === "compile" && active.cuTotal > 0) {
      detail = `compile ${active.cuDone}/${active.cuTotal} cu`;
    } else if (phase === "link") {
      detail = "linking (cargo may look idle)";
    }
  }
  const mins = Math.floor(snapshot.elapsedSec / 60);
  const secs = snapshot.elapsedSec % 60;
  const elapsed = mins > 0 ? `${mins}m${secs}s` : `${secs}s`;
  return `whisper-gpu: [${bar}] ${pct}% ${label} ${detail} (${elapsed})`;
}

/**
 * @param {GpuNativeBuildProgressSnapshot} snapshot
 * @param {{ reason?: string }} [opts]
 * @returns {string}
 */
export function formatGpuBuildDebugBlock(snapshot, opts = {}) {
  const lines = ["whisper-gpu: GPU build debug —"];
  if (opts.reason) lines.push(`  reason: ${opts.reason}`);
  lines.push(`  profile: ${snapshot.profile}`);
  lines.push(`  elapsed: ${snapshot.elapsedSec}s`);
  lines.push(`  prebuild cache warm: ${snapshot.prebuildWarm ? "yes" : "no"}`);
  if (!snapshot.prebuildWarm) {
    lines.push("  hint: run npm run prebuild:gpu-cuda to avoid 45–90+ min cold CUDA compile");
  }
  lines.push(
    `  processes: ${snapshot.runningProcesses.length > 0 ? snapshot.runningProcesses.join(", ") : "(none)"}`,
  );
  for (const crate of snapshot.crates) {
    lines.push(`  ${crate.label}:`);
    lines.push(`    phase: ${crate.phase}`);
    lines.push(`    out/build: ${crate.outBuildDir ?? "(not started)"}`);
    lines.push(`    ninja: ${crate.ninjaDone}/${crate.ninjaTotal}`);
    lines.push(`    cu: ${crate.cuDone}/${crate.cuTotal}  obj: ${crate.objCount}`);
    if (crate.lastNinjaTarget) {
      lines.push(`    last ninja target: ${crate.lastNinjaTarget}`);
    }
    if (crate.newestArtifactAgeSec != null) {
      lines.push(`    last artifact: ${crate.newestArtifactAgeSec}s ago`);
    }
    lines.push(`    ggml-cuda.lib: ${crate.ggmlCudaLibPresent ? "yes" : "no"}`);
  }
  lines.push("  diagnose: npm run diagnose:build-env");
  return lines.join("\n");
}

/**
 * @param {GpuNativeBuildProgressSnapshot} snapshot
 * @param {GpuNativeBuildProgressSnapshot | null} prevSnapshot
 * @returns {"active"|"slow_nvcc"|"stalled"|"linking"}
 */
export function detectGpuBuildStall(snapshot, prevSnapshot) {
  const active = snapshot.crates.find((c) => c.label === snapshot.activeCrate);
  if (!active || active.phase === "done") return "active";

  const procs = snapshot.runningProcesses.map((p) => p.toLowerCase());
  if (procs.some((p) => p === "link.exe" || p === "lib.exe")) return "linking";
  if (active.phase === "link") return "linking";

  const progressed =
    !prevSnapshot ||
    prevSnapshot.overallPercent !== snapshot.overallPercent ||
    snapshot.crates.some((crate, i) => {
      const prev = prevSnapshot.crates[i];
      if (!prev) return true;
      return (
        crate.phase !== prev.phase ||
        crate.ninjaDone !== prev.ninjaDone ||
        crate.cuDone !== prev.cuDone ||
        crate.objCount !== prev.objCount
      );
    });

  if (progressed) return "active";
  if (procs.some((p) => p === "nvcc.exe")) return "slow_nvcc";
  if (procs.some((p) => p === "ninja.exe" || p === "cmake.exe" || p === "cl.exe")) {
    return "active";
  }
  return "stalled";
}

/**
 * One poll tick for the tauri launcher (testable without spawn).
 * @param {object} opts
 * @param {string} opts.jarvisRoot
 * @param {"debug"|"release"} opts.profile
 * @param {number} opts.startedAt
 * @param {boolean} [opts.prebuildWarm]
 * @param {GpuNativeBuildProgressSnapshot | null} [opts.prevSnapshot]
 * @param {number} [opts.stallCount]
 * @param {() => string[]} [opts.probeProcesses]
 * @returns {{
 *   snapshot: GpuNativeBuildProgressSnapshot,
 *   barLine: string,
 *   debugBlock: string | null,
 *   stallState: ReturnType<typeof detectGpuBuildStall>,
 *   stallCount: number,
 *   phaseChanged: boolean,
 * }}
 */
export function tickGpuBuildProgressPoll(opts) {
  const {
    jarvisRoot,
    profile,
    startedAt,
    prebuildWarm = false,
    prevSnapshot = null,
    stallCount = 0,
    probeProcesses = probeGpuBuildProcesses,
  } = opts;

  const runningProcesses = probeProcesses();
  const snapshot = summarizeGpuNativeBuildProgress(jarvisRoot, profile, {
    startedAt,
    runningProcesses,
    prebuildWarm,
  });

  const stallState = detectGpuBuildStall(snapshot, prevSnapshot);
  const phaseChanged =
    !!prevSnapshot &&
    snapshot.crates.some((crate, i) => crate.phase !== prevSnapshot.crates[i]?.phase);

  let nextStallCount = stallCount;
  /** @type {string | null} */
  let debugBlock = null;

  if (phaseChanged) {
    debugBlock = formatGpuBuildDebugBlock(snapshot, { reason: "phase change" });
    nextStallCount = 0;
  } else if (stallState === "stalled") {
    nextStallCount = stallCount + 1;
    if (nextStallCount >= 2) {
      debugBlock = formatGpuBuildDebugBlock(snapshot, {
        reason: "no progress detected — possible hang (check .cargo-lock, stale cargo, generator mismatch)",
      });
    }
  } else if (stallState === "slow_nvcc" && stallCount === 0) {
    debugBlock = formatGpuBuildDebugBlock(snapshot, {
      reason: "nvcc running — large CUDA kernel compile (normal; may take several minutes per file)",
    });
    nextStallCount = 1;
  } else if (stallState === "linking" && phaseChanged === false && stallCount === 0) {
    debugBlock = formatGpuBuildDebugBlock(snapshot, {
      reason: "link step — cargo output may pause near the end",
    });
    nextStallCount = 1;
  } else if (stallState === "active") {
    nextStallCount = 0;
  }

  return {
    snapshot,
    barLine: formatGpuBuildProgressBar(snapshot),
    debugBlock,
    stallState,
    stallCount: nextStallCount,
    phaseChanged,
  };
}

/**
 * @param {ReturnType<typeof import("./diagnose-build-env.mjs").collectBuildEnvDiagnostics>} report
 * @returns {string[]}
 */
export function formatCondensedBuildEnvDiagnostics(report) {
  /** @type {string[]} */
  const lines = [];
  lines.push(
    `whisper-gpu: build-env — generator=${report.tauriCuda.CMAKE_GENERATOR} arch=${report.tauriCuda.CMAKE_CUDA_ARCHITECTURES ?? "?"} parallel=${report.tauriCuda.CMAKE_BUILD_PARALLEL_LEVEL ?? "?"}`,
  );
  lines.push(
    `whisper-gpu: prebuild cache ${report.gpuPrebuild.warm ? "warm" : "COLD"} (${report.gpuPrebuild.cacheRoot})`,
  );
  if (!report.gpuPrebuild.warm) {
    lines.push("whisper-gpu: >>> run npm run prebuild:gpu-cuda first to shorten cold GPU builds <<<");
  }
  for (const row of report.ggmlCudaArtifacts) {
    lines.push(`whisper-gpu: ${row.crate} ggml-cuda: ${row.present ? row.path : "not built yet"}`);
  }
  for (const w of report.warnings.slice(0, 3)) {
    lines.push(`whisper-gpu: warning — ${w}`);
  }
  return lines;
}
