import { describe, expect, it, afterEach } from "vitest";

import {
  applyCompilerCacheLauncherEnv,
  applyCudaBuildTuningEnv,
  applyCudaCmakeGenerator,
  CUDA_COMPATIBLE_GENERATORS,
  DEFAULT_CUDA_ARCHITECTURES,
  findCompilerCacheOnPath,
  findWindowsNinjaExe,
  formatCudaBuildProfileLog,
  resolveCudaArchitectures,
  resolveCudaBuildParallelLevel,
  resolveCudaCmakeGenerator,
  resolveJarvisCompilerCacheDir,
  resolveBackend,
} from "./detect.mjs";

describe("resolveCudaCmakeGenerator", () => {
  const prevOverride = process.env.JARVIS_CMAKE_GENERATOR;

  afterEach(() => {
    if (prevOverride === undefined) {
      delete process.env.JARVIS_CMAKE_GENERATOR;
    } else {
      process.env.JARVIS_CMAKE_GENERATOR = prevOverride;
    }
  });

  it("honors JARVIS_CMAKE_GENERATOR override", () => {
    process.env.JARVIS_CMAKE_GENERATOR = "NMake Makefiles";
    expect(resolveCudaCmakeGenerator()).toBe("NMake Makefiles");
  });

  it("prefers Ninja when ninja.exe is on PATH, else NMake", () => {
    delete process.env.JARVIS_CMAKE_GENERATOR;
    const expected = findWindowsNinjaExe() ? "Ninja" : "NMake Makefiles";
    expect(resolveCudaCmakeGenerator()).toBe(expected);
  });

  it("allows explicit Ninja via JARVIS_CMAKE_GENERATOR", () => {
    process.env.JARVIS_CMAKE_GENERATOR = "Ninja";
    expect(resolveCudaCmakeGenerator()).toBe("Ninja");
    expect(CUDA_COMPATIBLE_GENERATORS.has("Ninja")).toBe(true);
  });
});

describe("resolveCudaArchitectures", () => {
  const prev = process.env.JARVIS_CUDA_ARCH;

  afterEach(() => {
    if (prev === undefined) {
      delete process.env.JARVIS_CUDA_ARCH;
    } else {
      process.env.JARVIS_CUDA_ARCH = prev;
    }
  });

  it("defaults to RTX 40 sm_89", () => {
    delete process.env.JARVIS_CUDA_ARCH;
    expect(resolveCudaArchitectures()).toBe(DEFAULT_CUDA_ARCHITECTURES);
    expect(resolveCudaArchitectures()).toBe("89");
  });

  it("honors JARVIS_CUDA_ARCH override", () => {
    process.env.JARVIS_CUDA_ARCH = "86";
    expect(resolveCudaArchitectures()).toBe("86");
  });
});

describe("applyCudaBuildTuningEnv", () => {
  const prevArch = process.env.JARVIS_CUDA_ARCH;

  afterEach(() => {
    if (prevArch === undefined) {
      delete process.env.JARVIS_CUDA_ARCH;
    } else {
      process.env.JARVIS_CUDA_ARCH = prevArch;
    }
  });

  it("sets arch, parallel level, and skip-bindgen env", () => {
    /** @type {NodeJS.ProcessEnv} */
    const env = {};
    const tuning = applyCudaBuildTuningEnv(env);
    expect(env.CMAKE_CUDA_ARCHITECTURES).toBe("89");
    expect(env.CMAKE_BUILD_TYPE).toBe("Release");
    if (process.platform !== "win32") {
      expect(env.WHISPER_DONT_GENERATE_BINDINGS).toBe("1");
    } else {
      expect(env.WHISPER_DONT_GENERATE_BINDINGS).toBeUndefined();
    }
    expect(Number.parseInt(env.CMAKE_BUILD_PARALLEL_LEVEL ?? "", 10)).toBeGreaterThanOrEqual(1);
    expect(tuning.arch).toBe("89");
    expect(tuning.parallelLevel).toBe(resolveCudaBuildParallelLevel());
  });

  it("uses JARVIS_CUDA_ARCH when set on env object", () => {
    /** @type {NodeJS.ProcessEnv} */
    const env = { JARVIS_CUDA_ARCH: "75" };
    applyCudaBuildTuningEnv(env);
    expect(env.CMAKE_CUDA_ARCHITECTURES).toBe("75");
  });
});

describe("formatCudaBuildProfileLog", () => {
  it("includes arch, generator, parallel, and CUDA_PATH", () => {
    const line = formatCudaBuildProfileLog({
      arch: "89",
      generator: "NMake Makefiles",
      parallelLevel: "16",
      cudaRoot: "C:\\CUDA\\v13.2",
    });
    expect(line).toContain("arch=89");
    expect(line).toContain("generator=NMake Makefiles");
    expect(line).toContain("parallel=16");
    expect(line).toContain("CUDA_PATH=C:\\CUDA\\v13.2");
  });

  it("includes compiler-cache when set", () => {
    const line = formatCudaBuildProfileLog({
      arch: "89",
      generator: "Ninja",
      parallelLevel: "16",
      cudaRoot: "C:\\CUDA\\v13.2",
      compilerCache: "ccache",
    });
    expect(line).toContain("compiler-cache=ccache");
  });
});

describe("resolveJarvisCompilerCacheDir", () => {
  it("uses jarvis/.cache/ccache for ccache", () => {
    expect(resolveJarvisCompilerCacheDir("ccache")).toMatch(/[\\/]\.cache[\\/]ccache$/);
  });

  it("uses jarvis/.cache/sccache for sccache", () => {
    expect(resolveJarvisCompilerCacheDir("sccache")).toMatch(/[\\/]\.cache[\\/]sccache$/);
  });
});

describe("applyCompilerCacheLauncherEnv", () => {
  it("respects existing CMAKE_*_COMPILER_LAUNCHER env", () => {
    /** @type {NodeJS.ProcessEnv} */
    const env = { CMAKE_C_COMPILER_LAUNCHER: "mycache" };
    const r = applyCompilerCacheLauncherEnv(env, { warnIfMissing: false });
    expect(r.enabled).toBe(true);
    expect(r.launcher).toBe("mycache");
    expect(r.source).toBe("env");
    expect(env.CMAKE_CXX_COMPILER_LAUNCHER).toBeUndefined();
  });

  it("sets launcher + cache dir when ccache/sccache is on PATH", () => {
    const found = findCompilerCacheOnPath();
    /** @type {NodeJS.ProcessEnv} */
    const env = {};
    const r = applyCompilerCacheLauncherEnv(env, { warnIfMissing: false });
    if (!found) {
      expect(r.enabled).toBe(false);
      expect(env.CMAKE_C_COMPILER_LAUNCHER).toBeUndefined();
      return;
    }
    expect(r.enabled).toBe(true);
    if (process.platform === "win32") {
      expect(env.CMAKE_C_COMPILER_LAUNCHER).toBeUndefined();
      expect(env.CMAKE_CXX_COMPILER_LAUNCHER).toBeUndefined();
      expect(env.CMAKE_CUDA_COMPILER_LAUNCHER).toBeUndefined();
    } else {
      expect(env.CMAKE_C_COMPILER_LAUNCHER).toBe(found.launcher);
      expect(env.CMAKE_CXX_COMPILER_LAUNCHER).toBe(found.launcher);
      expect(env.CMAKE_CUDA_COMPILER_LAUNCHER).toBe(found.launcher);
    }
    const expectedDir = resolveJarvisCompilerCacheDir(found.launcher);
    if (found.launcher === "sccache") {
      expect(env.SCCACHE_DIR).toBe(expectedDir);
    } else {
      expect(env.CCACHE_DIR).toBe(expectedDir);
    }
  });
});

describe("applyCudaCmakeGenerator", () => {
  it("overrides stale Visual Studio generator", () => {
    /** @type {NodeJS.ProcessEnv} */
    const env = { CMAKE_GENERATOR: "Visual Studio 17 2022" };
    const r = applyCudaCmakeGenerator(env, { logPrefix: "test" });
    expect(env.CMAKE_GENERATOR).toBe(r.target);
    expect(CUDA_COMPATIBLE_GENERATORS.has(env.CMAKE_GENERATOR ?? "")).toBe(true);
    expect(r.overridden).toBe(true);
    expect(r.previous).toBe("Visual Studio 17 2022");
  });

  it("sets generator when unset", () => {
    /** @type {NodeJS.ProcessEnv} */
    const env = {};
    applyCudaCmakeGenerator(env, { logPrefix: "test" });
    expect(CUDA_COMPATIBLE_GENERATORS.has(env.CMAKE_GENERATOR ?? "")).toBe(true);
  });
});

describe("resolveBackend", () => {
  const prev = process.env.WHISPER_GPU_BACKEND;

  it("honors WHISPER_GPU_BACKEND=none override", () => {
    process.env.WHISPER_GPU_BACKEND = "none";
    const selected = resolveBackend();
    expect(selected.backend).toBe("none");
    expect(selected.forced).toBe(true);
    if (prev === undefined) {
      delete process.env.WHISPER_GPU_BACKEND;
    } else {
      process.env.WHISPER_GPU_BACKEND = prev;
    }
  });

  it("honors WHISPER_GPU_BACKEND=cuda override when forced", () => {
    process.env.WHISPER_GPU_BACKEND = "cuda";
    const selected = resolveBackend();
    expect(selected.backend).toBe("cuda");
    expect(selected.forced).toBe(true);
    if (prev === undefined) {
      delete process.env.WHISPER_GPU_BACKEND;
    } else {
      process.env.WHISPER_GPU_BACKEND = prev;
    }
  });
});
