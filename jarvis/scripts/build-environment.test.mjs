import path from "path";
import { fileURLToPath } from "url";
import { afterEach, describe, expect, it, vi } from "vitest";

import { prepareGpuNativeBuild, resolveBuildEnvironment } from "./build-environment.mjs";
import * as gpuPrebuild from "./gpu-prebuild.mjs";
import * as preflight from "./whisper-gpu/preflight.mjs";

const JARVIS_ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");

describe("resolveBuildEnvironment", () => {
  it("sync channel returns discovered CMAKE_GENERATOR when CUDA toolkit present", () => {
    if (process.platform !== "win32") return;
    const { discovered, env } = resolveBuildEnvironment({
      jarvisRoot: JARVIS_ROOT,
      channel: "sync",
      baseEnv: {},
      needsCuda: true,
    });
    expect(discovered.CMAKE_GENERATOR).toBeTruthy();
    expect(["Ninja", "NMake Makefiles"]).toContain(discovered.CMAKE_GENERATOR);
    expect(env.CMAKE_GENERATOR).toBe(discovered.CMAKE_GENERATOR);
  });

  it("cargo channel sets CARGO_TERM_PROGRESS", () => {
    const { env } = resolveBuildEnvironment({
      jarvisRoot: JARVIS_ROOT,
      channel: "cargo",
      baseEnv: {},
      needsCuda: false,
    });
    expect(env.CARGO_TERM_PROGRESS).toBe("always");
  });

  it("returns a copy of baseEnv — caller env object is not mutated", () => {
    const baseEnv = { FOO: "bar" };
    const { env } = resolveBuildEnvironment({
      jarvisRoot: JARVIS_ROOT,
      channel: "cargo",
      baseEnv,
      needsCuda: false,
    });
    expect(env).not.toBe(baseEnv);
    expect(env.CARGO_TERM_PROGRESS).toBe("always");
    expect(baseEnv.CARGO_TERM_PROGRESS).toBeUndefined();
  });
});

describe("prepareGpuNativeBuild", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("returns cacheWarm false and warns when prebuild cache is not warm", () => {
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});

    const result = prepareGpuNativeBuild(JARVIS_ROOT, { CMAKE_GENERATOR: "Ninja" });

    expect(result).toEqual({ seeded: [], purged: [], applied: 0, cacheWarm: false });
    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining("GPU prebuild cache not warm"),
    );
  });

  it("forwards profile to seedGpuPrebuildCache when cache is warm", () => {
    const seedSpy = vi.spyOn(gpuPrebuild, "seedGpuPrebuildCache").mockReturnValue({
      seeded: ["src-tauri/target/release/build/llama-cpp-sys-2-dead/out/build"],
      skipped: [],
      warnings: [],
    });

    const env = { JARVIS_GPU_PREBUILD_WARM: "1", CMAKE_GENERATOR: "Ninja", CMAKE_CUDA_COMPILER_LAUNCHER: "ccache" };
    prepareGpuNativeBuild(
      JARVIS_ROOT,
      env,
      { profile: "release" },
    );

    expect(seedSpy).toHaveBeenCalledWith(
      JARVIS_ROOT,
      expect.objectContaining({ profile: "release", generator: "Ninja" }),
    );
    if (process.platform === "win32") {
      expect(env.CMAKE_CUDA_COMPILER_LAUNCHER).toBeUndefined();
    }
  });

  it("defaults profile to debug when not specified", () => {
    const seedSpy = vi.spyOn(gpuPrebuild, "seedGpuPrebuildCache").mockReturnValue({
      seeded: [],
      skipped: [],
      warnings: [],
    });

    prepareGpuNativeBuild(JARVIS_ROOT, {
      JARVIS_GPU_PREBUILD_WARM: "1",
      CMAKE_GENERATOR: "Ninja",
    });

    expect(seedSpy).toHaveBeenCalledWith(
      JARVIS_ROOT,
      expect.objectContaining({ profile: "debug" }),
    );
  });

  it("skips generator mismatch warning when warnGeneratorMismatch is false", () => {
    const warnSpy = vi.spyOn(preflight, "warnIfCmakeGeneratorMismatch").mockImplementation(() => {});

    prepareGpuNativeBuild(
      JARVIS_ROOT,
      { CMAKE_GENERATOR: "Ninja" },
      { warnGeneratorMismatch: false },
    );

    expect(warnSpy).not.toHaveBeenCalled();
  });
});
