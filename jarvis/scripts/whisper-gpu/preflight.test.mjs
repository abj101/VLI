import fs from "fs";
import os from "os";
import path from "path";
import { fileURLToPath } from "url";
import { afterEach, describe, expect, it } from "vitest";

import {
  cargoLockPaths,
  checkCargoBuildLock,
  clearGpuSysCmakeCacheOnGeneratorMismatch,
  clearIncompleteGpuSysCmakeCache,
  clearWhisperRsSysBuildCacheOnGeneratorMismatch,
  isLikelyFirstCudaWhisperBuild,
  summarizeGgmlCudaArtifacts,
  summarizeGpuNativeBuildActivity,
  summarizeWhisperRsSysBuildActivity,
} from "./preflight.mjs";
import { collectBuildEnvDiagnostics } from "../diagnose-build-env.mjs";

const JARVIS_ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "..");

describe("cargoLockPaths", () => {
  it("includes debug and release target locks", () => {
    const paths = cargoLockPaths(JARVIS_ROOT);
    expect(paths.some((p) => p.endsWith("target\\debug\\.cargo-lock") || p.endsWith("target/debug/.cargo-lock"))).toBe(
      true,
    );
  });
});

describe("checkCargoBuildLock", () => {
  /** @type {string | null} */
  let tmpRoot = null;

  afterEach(() => {
    if (tmpRoot) {
      fs.rmSync(tmpRoot, { recursive: true, force: true });
      tmpRoot = null;
    }
  });

  it("removes stale lock files when no build is active", () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "jarvis-lock-"));
    const targetDebug = path.join(tmpRoot, "src-tauri", "target", "debug");
    fs.mkdirSync(targetDebug, { recursive: true });
    const lockFile = path.join(targetDebug, ".cargo-lock");
    fs.writeFileSync(lockFile, "");

    const r = checkCargoBuildLock(tmpRoot);
    expect(r.blocked).toBe(false);
    expect(fs.existsSync(lockFile)).toBe(false);
    expect(r.cleared?.length).toBeGreaterThan(0);
  });

  it("returns boolean for real jarvis root", () => {
    const r = checkCargoBuildLock(JARVIS_ROOT);
    expect(typeof r.blocked).toBe("boolean");
  });
});

describe("isLikelyFirstCudaWhisperBuild", () => {
  it("returns boolean without throwing", () => {
    expect(typeof isLikelyFirstCudaWhisperBuild(JARVIS_ROOT)).toBe("boolean");
  });
});

describe("clearGpuSysCmakeCacheOnGeneratorMismatch", () => {
  /** @type {string | null} */
  let tmpRoot = null;

  afterEach(() => {
    if (tmpRoot) {
      fs.rmSync(tmpRoot, { recursive: true, force: true });
      tmpRoot = null;
    }
  });

  it("removes whisper-rs-sys out/build when cached generator differs", () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "jarvis-cmake-"));
    const outBuild = path.join(
      tmpRoot,
      "src-tauri",
      "target",
      "debug",
      "build",
      "whisper-rs-sys-deadbeef",
      "out",
      "build",
    );
    fs.mkdirSync(outBuild, { recursive: true });
    fs.writeFileSync(
      path.join(outBuild, "CMakeCache.txt"),
      "CMAKE_GENERATOR:INTERNAL=Visual Studio 17 2022\n",
    );

    const r = clearGpuSysCmakeCacheOnGeneratorMismatch(tmpRoot, "NMake Makefiles");
    expect(r.cleared.length).toBe(1);
    expect(fs.existsSync(outBuild)).toBe(false);
  });

  it("removes llama-cpp-sys-2 out/build when cached generator differs", () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "jarvis-cmake-llama-"));
    const outBuild = path.join(
      tmpRoot,
      "src-tauri",
      "target",
      "debug",
      "build",
      "llama-cpp-sys-2-cafebabe",
      "out",
      "build",
    );
    fs.mkdirSync(outBuild, { recursive: true });
    fs.writeFileSync(
      path.join(outBuild, "CMakeCache.txt"),
      "CMAKE_GENERATOR:INTERNAL=Visual Studio 17 2022\n",
    );

    const r = clearGpuSysCmakeCacheOnGeneratorMismatch(tmpRoot, "Ninja");
    expect(r.cleared.length).toBe(1);
    expect(fs.existsSync(outBuild)).toBe(false);
  });

  it("detects Ninja cache when CMAKE_CUDA_COMPILER is present", () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "jarvis-cmake-ninja-cuda-"));
    const outBuild = path.join(
      tmpRoot,
      "src-tauri",
      "target",
      "debug",
      "build",
      "whisper-rs-sys-deadbeef",
      "out",
      "build",
    );
    fs.mkdirSync(outBuild, { recursive: true });
    fs.writeFileSync(
      path.join(outBuild, "CMakeCache.txt"),
      [
        "CMAKE_GENERATOR:INTERNAL=Ninja",
        "CMAKE_CUDA_COMPILER:FILEPATH=C:/CUDA/bin/nvcc.exe",
        "CMAKE_MAKE_PROGRAM:FILEPATH=C:/tools/ninja.exe",
      ].join("\n"),
    );

    const r = clearGpuSysCmakeCacheOnGeneratorMismatch(tmpRoot, "NMake Makefiles");
    expect(r.cleared.length).toBe(1);
  });

  it("clearWhisperRsSysBuildCacheOnGeneratorMismatch alias matches", () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "jarvis-cmake-alias-"));
    const outBuild = path.join(
      tmpRoot,
      "src-tauri",
      "target",
      "debug",
      "build",
      "whisper-rs-sys-deadbeef",
      "out",
      "build",
    );
    fs.mkdirSync(outBuild, { recursive: true });
    fs.writeFileSync(
      path.join(outBuild, "CMakeCache.txt"),
      "CMAKE_GENERATOR:INTERNAL=Visual Studio 17 2022\n",
    );

    const r = clearWhisperRsSysBuildCacheOnGeneratorMismatch(
      tmpRoot,
      "NMake Makefiles",
    );
    expect(r.cleared.length).toBe(1);
  });
});

describe("clearIncompleteGpuSysCmakeCache", () => {
  /** @type {string | null} */
  let tmpRoot = null;

  afterEach(() => {
    if (tmpRoot) {
      fs.rmSync(tmpRoot, { recursive: true, force: true });
      tmpRoot = null;
    }
  });

  it("removes llama-cpp-sys out/build when CMakeCache exists without build.ninja", () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "jarvis-cmake-incomplete-"));
    const outBuild = path.join(
      tmpRoot,
      "src-tauri",
      "target",
      "debug",
      "build",
      "llama-cpp-sys-2-deadbeef",
      "out",
      "build",
    );
    fs.mkdirSync(outBuild, { recursive: true });
    fs.writeFileSync(
      path.join(outBuild, "CMakeCache.txt"),
      "CMAKE_GENERATOR:UNINITIALIZED=Ninja\n",
    );

    const r = clearIncompleteGpuSysCmakeCache(tmpRoot);
    expect(r.cleared.length).toBe(1);
    expect(fs.existsSync(outBuild)).toBe(false);
  });

  it("keeps complete Ninja trees that have build.ninja", () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "jarvis-cmake-complete-"));
    const outBuild = path.join(
      tmpRoot,
      "src-tauri",
      "target",
      "debug",
      "build",
      "llama-cpp-sys-2-cafebabe",
      "out",
      "build",
    );
    fs.mkdirSync(outBuild, { recursive: true });
    fs.writeFileSync(
      path.join(outBuild, "CMakeCache.txt"),
      "CMAKE_GENERATOR:INTERNAL=Ninja\n",
    );
    fs.writeFileSync(path.join(outBuild, "build.ninja"), "# ninja\n");

    const r = clearIncompleteGpuSysCmakeCache(tmpRoot);
    expect(r.cleared.length).toBe(0);
    expect(fs.existsSync(outBuild)).toBe(true);
  });
});

describe("summarizeGgmlCudaArtifacts", () => {
  it("returns both GPU crates without throwing", () => {
    const rows = summarizeGgmlCudaArtifacts(JARVIS_ROOT);
    expect(rows.length).toBe(2);
    expect(rows.map((r) => r.label)).toEqual(["whisper-rs-sys", "llama-cpp-sys"]);
  });
});

describe("collectBuildEnvDiagnostics", () => {
  it("returns structured env report without throwing", () => {
    const report = collectBuildEnvDiagnostics(JARVIS_ROOT);
    expect(report.platform).toBe(process.platform);
    expect(report.tauriCuda).toHaveProperty("CMAKE_GENERATOR");
    expect(Array.isArray(report.ggmlCudaArtifacts)).toBe(true);
    expect(report.ggmlCudaArtifacts.length).toBe(2);
    expect(Array.isArray(report.warnings)).toBe(true);
  });
});

describe("summarizeWhisperRsSysBuildActivity", () => {
  it("returns numeric fields without throwing", () => {
    const s = summarizeWhisperRsSysBuildActivity(JARVIS_ROOT);
    expect(typeof s.artifactCount).toBe("number");
    expect(s.newestAgeSec === null || typeof s.newestAgeSec === "number").toBe(true);
  });
});

describe("summarizeGpuNativeBuildActivity", () => {
  it("returns per-crate activity without throwing", () => {
    const s = summarizeGpuNativeBuildActivity(JARVIS_ROOT);
    expect(typeof s.artifactCount).toBe("number");
    expect(Array.isArray(s.crates)).toBe(true);
    for (const crate of s.crates) {
      expect(typeof crate.label).toBe("string");
      expect(typeof crate.artifactCount).toBe("number");
      expect(crate.newestAgeSec === null || typeof crate.newestAgeSec === "number").toBe(
        true,
      );
    }
  });
});
