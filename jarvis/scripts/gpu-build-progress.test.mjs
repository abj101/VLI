import fs from "fs";
import os from "os";
import path from "path";
import { afterEach, describe, expect, it } from "vitest";

import {
  detectGpuBuildStall,
  formatGpuBuildProgressBar,
  formatGpuBuildDebugBlock,
  inspectGpuSysCrateBuild,
  parseNinjaLog,
  summarizeGpuNativeBuildProgress,
  tickGpuBuildProgressPoll,
} from "./gpu-build-progress.mjs";

describe("inspectGpuSysCrateBuild", () => {
  /** @type {string | null} */
  let tmpRoot = null;

  afterEach(() => {
    if (tmpRoot) {
      fs.rmSync(tmpRoot, { recursive: true, force: true });
      tmpRoot = null;
    }
  });

  it("returns phase configure when only CMakeCache.txt exists", () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "jarvis-gpu-progress-"));
    const outBuild = path.join(tmpRoot, "out", "build");
    fs.mkdirSync(outBuild, { recursive: true });
    fs.writeFileSync(path.join(outBuild, "CMakeCache.txt"), "CMAKE_GENERATOR:INTERNAL=Ninja\n");

    const snap = inspectGpuSysCrateBuild(outBuild, "whisper-rs-sys");
    expect(snap.phase).toBe("configure");
    expect(snap.ggmlCudaLibPresent).toBe(false);
  });

  it("returns phase compile when build.ninja exists", () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "jarvis-gpu-progress-compile-"));
    const outBuild = path.join(tmpRoot, "out", "build");
    fs.mkdirSync(outBuild, { recursive: true });
    fs.writeFileSync(path.join(outBuild, "CMakeCache.txt"), "CMAKE_GENERATOR:INTERNAL=Ninja\n");
    fs.writeFileSync(path.join(outBuild, "build.ninja"), "build kernel.obj: cuda kernel.cu\n");

    const snap = inspectGpuSysCrateBuild(outBuild, "whisper-rs-sys");
    expect(snap.phase).toBe("compile");
  });

  it("returns phase done when ggml-cuda.lib exists", () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "jarvis-gpu-progress-done-"));
    const outBuild = path.join(tmpRoot, "out", "build");
    const libDir = path.join(outBuild, "ggml", "src", "ggml-cuda");
    fs.mkdirSync(libDir, { recursive: true });
    fs.writeFileSync(path.join(libDir, "ggml-cuda.lib"), "");

    const snap = inspectGpuSysCrateBuild(outBuild, "whisper-rs-sys");
    expect(snap.phase).toBe("done");
    expect(snap.cratePercent).toBe(100);
  });

  it("counts cu/obj compile ratio under ggml-cuda", () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "jarvis-gpu-progress-cu-"));
    const ggmlCuda = path.join(tmpRoot, "out", "build", "ggml", "src", "ggml-cuda");
    fs.mkdirSync(ggmlCuda, { recursive: true });
    fs.writeFileSync(path.join(ggmlCuda, "a.cu"), "");
    fs.writeFileSync(path.join(ggmlCuda, "b.cu"), "");
    fs.writeFileSync(path.join(ggmlCuda, "a.obj"), "");
    fs.writeFileSync(path.join(tmpRoot, "out", "build", "CMakeCache.txt"), "");
    fs.writeFileSync(path.join(tmpRoot, "out", "build", "build.ninja"), "build a.obj: cuda a.cu\n");

    const snap = inspectGpuSysCrateBuild(path.join(tmpRoot, "out", "build"), "whisper-rs-sys");
    expect(snap.cuTotal).toBe(2);
    expect(snap.cuDone).toBe(1);
    expect(snap.objCount).toBeGreaterThanOrEqual(1);
  });
});

describe("parseNinjaLog", () => {
  it("parses completed rules and last target from ninja log v5", () => {
    const text = [
      "# ninja log v5",
      "1 2 3 4 ggml-cuda/foo.obj",
      "5 6 7 8 ggml-cuda/bar.obj",
    ].join("\n");
    const parsed = parseNinjaLog(text);
    expect(parsed.done).toBe(2);
    expect(parsed.lastTarget).toBe("ggml-cuda/bar.obj");
  });
});

describe("summarizeGpuNativeBuildProgress", () => {
  /** @type {string | null} */
  let tmpRoot = null;

  afterEach(() => {
    if (tmpRoot) {
      fs.rmSync(tmpRoot, { recursive: true, force: true });
      tmpRoot = null;
    }
  });

  it("aggregates overallPercent across whisper and llama crates", () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "jarvis-gpu-progress-sum-"));
    const buildRoot = path.join(tmpRoot, "src-tauri", "target", "debug", "build");

    const whisperOut = path.join(buildRoot, "whisper-rs-sys-abc", "out", "build");
    const llamaOut = path.join(buildRoot, "llama-cpp-sys-2-def", "out", "build");
    fs.mkdirSync(whisperOut, { recursive: true });
    fs.mkdirSync(llamaOut, { recursive: true });
    fs.writeFileSync(path.join(whisperOut, "CMakeCache.txt"), "");
    fs.writeFileSync(path.join(whisperOut, "build.ninja"), "build x: cuda x.cu\n");
    const llamaLib = path.join(llamaOut, "ggml", "src", "ggml-cuda", "ggml-cuda.lib");
    fs.mkdirSync(path.dirname(llamaLib), { recursive: true });
    fs.writeFileSync(llamaLib, "");

    const snap = summarizeGpuNativeBuildProgress(tmpRoot, "debug", {
      startedAt: Date.now() - 30_000,
      runningProcesses: [],
      prebuildWarm: false,
    });
    expect(snap.crates).toHaveLength(2);
    expect(snap.overallPercent).toBeGreaterThan(0);
    expect(snap.overallPercent).toBeLessThan(100);
    expect(snap.elapsedSec).toBeGreaterThanOrEqual(30);
  });

  it("prefers the whisper-rs-sys dir with ggml-cuda.lib when multiple hashed dirs exist", () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "jarvis-gpu-progress-dup-"));
    const buildRoot = path.join(tmpRoot, "src-tauri", "target", "release", "build");

    const staleOut = path.join(buildRoot, "whisper-rs-sys-039c414732ab9610", "out", "build");
    fs.mkdirSync(staleOut, { recursive: true });

    const warmOut = path.join(buildRoot, "whisper-rs-sys-9ce17f8a8c45cc9d", "out", "build");
    const warmLib = path.join(warmOut, "ggml", "src", "ggml-cuda", "ggml-cuda.lib");
    fs.mkdirSync(path.dirname(warmLib), { recursive: true });
    fs.writeFileSync(warmLib, "");

    const llamaOut = path.join(buildRoot, "llama-cpp-sys-2-836551779c43c4b5", "out", "build");
    const llamaLib = path.join(llamaOut, "ggml", "src", "ggml-cuda", "ggml-cuda.lib");
    fs.mkdirSync(path.dirname(llamaLib), { recursive: true });
    fs.writeFileSync(llamaLib, "");

    const snap = summarizeGpuNativeBuildProgress(tmpRoot, "release", { prebuildWarm: true });
    expect(snap.overallPercent).toBe(100);
    expect(snap.crates[0].phase).toBe("done");
    expect(snap.crates[0].ggmlCudaLibPresent).toBe(true);
  });
});

describe("formatGpuBuildProgressBar", () => {
  it("includes percent and active crate label", () => {
    const line = formatGpuBuildProgressBar({
      profile: "release",
      elapsedSec: 125,
      overallPercent: 42,
      activeCrate: "whisper-rs-sys",
      runningProcesses: [],
      prebuildWarm: true,
      crates: [
        {
          phase: "compile",
          label: "whisper-rs-sys",
          outBuildDir: "/tmp",
          cuTotal: 10,
          cuDone: 4,
          objCount: 4,
          ninjaDone: 118,
          ninjaTotal: 240,
          lastNinjaTarget: "ggml-cuda/foo.obj",
          ggmlCudaLibPresent: false,
          newestArtifactAgeSec: 5,
          cratePercent: 42,
        },
        {
          phase: "pending",
          label: "llama-cpp-sys",
          outBuildDir: null,
          cuTotal: 0,
          cuDone: 0,
          objCount: 0,
          ninjaDone: 0,
          ninjaTotal: 0,
          lastNinjaTarget: null,
          ggmlCudaLibPresent: false,
          newestArtifactAgeSec: null,
          cratePercent: 0,
        },
      ],
    });
    expect(line).toContain("42%");
    expect(line).toContain("whisper-rs-sys");
    expect(line).toContain("118/240 ninja");
  });
});

describe("detectGpuBuildStall", () => {
  const baseSnapshot = {
    profile: /** @type {"debug"} */ ("debug"),
    elapsedSec: 600,
    overallPercent: 20,
    activeCrate: "whisper-rs-sys",
    runningProcesses: [],
    prebuildWarm: false,
    crates: [
      {
        phase: /** @type {"compile"} */ ("compile"),
        label: "whisper-rs-sys",
        outBuildDir: "/x",
        cuTotal: 10,
        cuDone: 2,
        objCount: 2,
        ninjaDone: 10,
        ninjaTotal: 100,
        lastNinjaTarget: "a.obj",
        ggmlCudaLibPresent: false,
        newestArtifactAgeSec: 300,
        cratePercent: 20,
      },
      {
        phase: /** @type {"pending"} */ ("pending"),
        label: "llama-cpp-sys",
        outBuildDir: null,
        cuTotal: 0,
        cuDone: 0,
        objCount: 0,
        ninjaDone: 0,
        ninjaTotal: 0,
        lastNinjaTarget: null,
        ggmlCudaLibPresent: false,
        newestArtifactAgeSec: null,
        cratePercent: 0,
      },
    ],
  };

  it("returns slow_nvcc when counts unchanged but nvcc is running", () => {
    const snap = {
      ...baseSnapshot,
      runningProcesses: ["nvcc.exe"],
    };
    expect(detectGpuBuildStall(snap, baseSnapshot)).toBe("slow_nvcc");
  });

  it("returns stalled when no progress and no build processes", () => {
    expect(detectGpuBuildStall(baseSnapshot, baseSnapshot)).toBe("stalled");
  });

  it("returns linking when link.exe is running", () => {
    const snap = {
      ...baseSnapshot,
      runningProcesses: ["link.exe"],
    };
    expect(detectGpuBuildStall(snap, baseSnapshot)).toBe("linking");
  });
});

describe("formatGpuBuildDebugBlock", () => {
  it("includes prebuild hint when cache is cold", () => {
    const block = formatGpuBuildDebugBlock({
      profile: "release",
      elapsedSec: 10,
      overallPercent: 5,
      activeCrate: "whisper-rs-sys",
      runningProcesses: [],
      prebuildWarm: false,
      crates: [],
    });
    expect(block).toContain("prebuild:gpu-cuda");
  });
});

describe("tickGpuBuildProgressPoll", () => {
  /** @type {string | null} */
  let tmpRoot = null;

  afterEach(() => {
    if (tmpRoot) {
      fs.rmSync(tmpRoot, { recursive: true, force: true });
      tmpRoot = null;
    }
  });

  it("returns a bar line for an in-progress tree", () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "jarvis-gpu-progress-tick-"));
    const outBuild = path.join(
      tmpRoot,
      "src-tauri",
      "target",
      "release",
      "build",
      "whisper-rs-sys-abc",
      "out",
      "build",
    );
    fs.mkdirSync(outBuild, { recursive: true });
    fs.writeFileSync(path.join(outBuild, "CMakeCache.txt"), "");
    fs.writeFileSync(path.join(outBuild, "build.ninja"), "build a.obj: cuda a.cu\n");

    const tick = tickGpuBuildProgressPoll({
      jarvisRoot: tmpRoot,
      profile: "release",
      startedAt: Date.now() - 5000,
      prebuildWarm: false,
      probeProcesses: () => [],
    });

    expect(tick.barLine).toMatch(/whisper-gpu: \[/);
    expect(tick.snapshot.overallPercent).toBeGreaterThanOrEqual(0);
  });
});
