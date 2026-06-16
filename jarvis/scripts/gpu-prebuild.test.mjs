import fs from "fs";
import os from "os";
import path from "path";
import { fileURLToPath } from "url";
import { afterEach, describe, expect, it } from "vitest";

import { isCmakeCacheWellFormed } from "./gpu-native-sys.mjs";
import {
  buildCmakeConfigureArgv,
  cmakeCacheDefineArg,
  isGpuPrebuildCacheWarm,
  isGpuPrebuildEntryWarm,
  llamaCudaPrebuildDefines,
  readCargoLockSysVersions,
  resolveGpuPrebuildCacheRoot,
  resolveGpuPrebuildEntryDirs,
  rewriteCmakeHomeDirectory,
  rewritePathPrefixesInSeededBuildTree,
  seededCmakeBuildNeedsRegen,
  summarizeGpuPrebuildCache,
  writeGpuPrebuildManifest,
} from "./gpu-prebuild.mjs";

const JARVIS_ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");

describe("buildCmakeConfigureArgv", () => {
  it("keeps MSVC paths with spaces in a single -D argument", () => {
    const cl =
      "C:\\Program Files\\Microsoft Visual Studio\\2022\\BuildTools\\VC\\Tools\\MSVC\\14.44.35207\\bin\\Hostx64\\x64\\cl.exe";
    const argv = buildCmakeConfigureArgv({
      sourceDir: "C:\\src\\whisper.cpp",
      buildDir: "C:\\cache\\whisper\\build",
      env: {
        CMAKE_GENERATOR: "Ninja",
        CMAKE_C_COMPILER: cl,
        CMAKE_CXX_COMPILER: cl,
        CMAKE_CUDA_HOST_COMPILER: cl,
      },
      defines: { GGML_CUDA: "ON" },
    });
    const hostArg = argv.find((a) => a.startsWith("-DCMAKE_CUDA_HOST_COMPILER="));
    expect(hostArg).toBe(`-DCMAKE_CUDA_HOST_COMPILER=${cl.replace(/\\/g, "/")}`);
  });

  it("cmakeCacheDefineArg normalizes Windows paths for CMake", () => {
    expect(cmakeCacheDefineArg("CMAKE_RC_COMPILER", "C:\\Kits\\10\\bin\\rc.exe")).toBe(
      "-DCMAKE_RC_COMPILER=C:/Kits/10/bin/rc.exe",
    );
  });

  it("cmakeCacheDefineArg returns null for empty values", () => {
    expect(cmakeCacheDefineArg("CMAKE_C_COMPILER", "")).toBeNull();
    expect(cmakeCacheDefineArg("CMAKE_C_COMPILER", "cl.exe")).toBe("-DCMAKE_C_COMPILER=cl.exe");
  });
});

describe("gpu-prebuild paths", () => {
  it("resolves cache root under jarvis/.cache/gpu-prebuild/<arch>", () => {
    const root = resolveGpuPrebuildCacheRoot(JARVIS_ROOT, "89");
    expect(root).toBe(path.join(JARVIS_ROOT, ".cache", "gpu-prebuild", "89"));
  });

  it("resolves whisper/llama entry dirs", () => {
    const cacheRoot = resolveGpuPrebuildCacheRoot(JARVIS_ROOT, "89");
    const whisper = resolveGpuPrebuildEntryDirs("whisper", cacheRoot);
    expect(whisper.buildDir).toContain(path.join("whisper", "build"));
    expect(whisper.sourceDir).toContain(path.join("whisper", "source"));
  });
});

describe("readCargoLockSysVersions", () => {
  it("reads whisper-rs-sys and llama-cpp-sys-2 versions from Cargo.lock", () => {
    const versions = readCargoLockSysVersions(JARVIS_ROOT);
    expect(versions).not.toBeNull();
    expect(versions?.whisperRsSys).toMatch(/^\d+\./);
    expect(versions?.llamaCppSys2).toMatch(/^\d+\./);
  });
});

describe("gpu prebuild manifest + warm checks", () => {
  /** @type {string | null} */
  let tmpRoot = null;

  afterEach(() => {
    if (tmpRoot && fs.existsSync(tmpRoot)) {
      fs.rmSync(tmpRoot, { recursive: true, force: true });
    }
    tmpRoot = null;
  });

  it("isGpuPrebuildEntryWarm is false without ggml-cuda lib", () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "jarvis-gpu-prebuild-"));
    const buildDir = path.join(tmpRoot, "build");
    fs.mkdirSync(buildDir, { recursive: true });
    expect(isGpuPrebuildEntryWarm(buildDir)).toBe(false);
  });

  it("isGpuPrebuildCacheWarm requires manifest + both entries", () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "jarvis-gpu-prebuild-"));
    const cacheRoot = path.join(tmpRoot, ".cache", "gpu-prebuild", "89");
    const versions = readCargoLockSysVersions(JARVIS_ROOT);
    expect(versions).not.toBeNull();

    writeGpuPrebuildManifest(cacheRoot, {
      arch: "89",
      generator: "Ninja",
      profile: "Release",
      whisperRsSysVersion: versions.whisperRsSys,
      llamaCppSysVersion: versions.llamaCppSys2,
      createdAt: new Date().toISOString(),
    });

    expect(isGpuPrebuildCacheWarm(tmpRoot, "89", "Ninja")).toBe(false);

    const whisperBuild = resolveGpuPrebuildEntryDirs("whisper", cacheRoot).buildDir;
    const llamaBuild = resolveGpuPrebuildEntryDirs("llama", cacheRoot).buildDir;
    fs.mkdirSync(path.join(whisperBuild, "ggml", "src", "ggml-cuda"), { recursive: true });
    fs.mkdirSync(path.join(llamaBuild, "ggml", "src", "ggml-cuda"), { recursive: true });
    fs.writeFileSync(path.join(whisperBuild, "ggml", "src", "ggml-cuda", "ggml-cuda.lib"), "");
    fs.writeFileSync(path.join(llamaBuild, "ggml", "src", "ggml-cuda", "ggml-cuda.lib"), "");

    expect(isGpuPrebuildCacheWarm(tmpRoot, "89", "Ninja")).toBe(true);
    expect(isGpuPrebuildCacheWarm(tmpRoot, "89", "NMake Makefiles")).toBe(false);
  });

  it("rewriteCmakeHomeDirectory updates CMAKE_HOME_DIRECTORY and CMAKE_CACHEFILE_DIR", () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "jarvis-gpu-prebuild-"));
    const buildDir = path.join(tmpRoot, "build");
    fs.mkdirSync(buildDir, { recursive: true });
    const cachePath = path.join(buildDir, "CMakeCache.txt");
    fs.writeFileSync(
      cachePath,
      [
        "CMAKE_HOME_DIRECTORY:INTERNAL=C:/old/whisper.cpp",
        "CMAKE_CACHEFILE_DIR:INTERNAL=C:/old/cache/build",
        "CMAKE_INSTALL_PREFIX:INTERNAL=C:/Program Files (x86)/llama.cpp",
      ].join("\n"),
      "utf8",
    );
    const newHome = path.join(tmpRoot, "out", "whisper.cpp");
    expect(rewriteCmakeHomeDirectory(buildDir, newHome)).toBe(true);
    const text = fs.readFileSync(cachePath, "utf8");
    const normalizedHome = path.resolve(newHome).replace(/\\/g, "/");
    const normalizedBuild = path.resolve(buildDir).replace(/\\/g, "/");
    const normalizedInstall = path.resolve(buildDir, "..").replace(/\\/g, "/");
    expect(text).toContain(normalizedHome);
    expect(text).toContain(normalizedBuild);
    expect(text).toContain(normalizedInstall);
  });

  it("rewriteSeededCmakeCache repairs legacy corrupt CMAKE_BUILD_TYPE entries", () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "jarvis-gpu-prebuild-"));
    const buildDir = path.join(tmpRoot, "build");
    fs.mkdirSync(buildDir, { recursive: true });
    const cachePath = path.join(buildDir, "CMakeCache.txt");
    fs.writeFileSync(
      cachePath,
      [
        "CMAKE_HOME_DIRECTORY:INTERNAL=C:/old/llama.cpp",
        "CMAKE_CACHEFILE_DIR:INTERNAL=C:/old/cache/build",
        "CMAKE_INSTALL_PREFIX:INTERNAL=C:/old/out",
        "CMAKE_GENERATOR:INTERNAL=Ninja",
        "CMAKE_BUILD_TYPE=Release",
      ].join("\n"),
      "utf8",
    );
    const newHome = path.join(tmpRoot, "llama.cpp");
    rewriteCmakeHomeDirectory(buildDir, newHome);
    const text = fs.readFileSync(cachePath, "utf8");
    expect(isCmakeCacheWellFormed(text)).toBe(true);
    expect(text).toContain("CMAKE_BUILD_TYPE:STRING=Release");
  });

  it("rewriteSeededCmakeCache strips Windows compiler launchers from copied cache", () => {
    if (process.platform !== "win32") return;
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "jarvis-gpu-prebuild-"));
    const buildDir = path.join(tmpRoot, "build");
    fs.mkdirSync(buildDir, { recursive: true });
    const cachePath = path.join(buildDir, "CMakeCache.txt");
    fs.writeFileSync(
      cachePath,
      [
        "CMAKE_HOME_DIRECTORY:INTERNAL=C:/old/llama.cpp",
        "CMAKE_CACHEFILE_DIR:INTERNAL=C:/old/cache/build",
        "CMAKE_INSTALL_PREFIX:INTERNAL=C:/old/out",
        "CMAKE_CUDA_COMPILER_LAUNCHER:STRING=ccache",
        "CMAKE_C_COMPILER_LAUNCHER:STRING=ccache",
      ].join("\n"),
      "utf8",
    );
    const newHome = path.join(tmpRoot, "llama.cpp");
    rewriteCmakeHomeDirectory(buildDir, newHome);
    const text = fs.readFileSync(cachePath, "utf8");
    expect(text).not.toContain("CMAKE_CUDA_COMPILER_LAUNCHER");
    expect(text).not.toContain("CMAKE_C_COMPILER_LAUNCHER");
  });
});

describe("seededCmakeBuildNeedsRegen", () => {
  /** @type {string | null} */
  let tmpRoot = null;

  afterEach(() => {
    if (tmpRoot && fs.existsSync(tmpRoot)) {
      fs.rmSync(tmpRoot, { recursive: true, force: true });
    }
    tmpRoot = null;
  });

  it("returns true when BUILD_SHARED_LIBS is ON or ggml DLLs are installed", () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "jarvis-gpu-seed-"));
    const outDir = path.join(tmpRoot, "out");
    const buildDir = path.join(outDir, "build");
    fs.mkdirSync(path.join(outDir, "bin"), { recursive: true });
    fs.mkdirSync(buildDir, { recursive: true });
    fs.writeFileSync(
      path.join(buildDir, "CMakeCache.txt"),
      [
        "CMAKE_CACHEFILE_DIR:INTERNAL=" + path.resolve(buildDir).replace(/\\/g, "/"),
        "CMAKE_INSTALL_PREFIX:INTERNAL=" + path.resolve(outDir).replace(/\\/g, "/"),
        "BUILD_SHARED_LIBS:BOOL=ON",
      ].join("\n"),
      "utf8",
    );
    fs.writeFileSync(path.join(buildDir, "build.ninja"), "# SHARED_LIBRARY target ggml-base\n", "utf8");
    expect(seededCmakeBuildNeedsRegen(buildDir)).toBe(true);

    fs.writeFileSync(
      path.join(buildDir, "CMakeCache.txt"),
      [
        "CMAKE_CACHEFILE_DIR:INTERNAL=" + path.resolve(buildDir).replace(/\\/g, "/"),
        "CMAKE_INSTALL_PREFIX:INTERNAL=" + path.resolve(outDir).replace(/\\/g, "/"),
        "BUILD_SHARED_LIBS:BOOL=OFF",
      ].join("\n"),
      "utf8",
    );
    fs.writeFileSync(path.join(buildDir, "build.ninja"), "# STATIC_LIBRARY target ggml-base\n", "utf8");
    fs.writeFileSync(path.join(outDir, "bin", "ggml-base.dll"), "", "utf8");
    expect(seededCmakeBuildNeedsRegen(buildDir)).toBe(true);
  });

  it("returns false for warm static CUDA tree even when ninja comments mention SHARED_LIBRARY", () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "jarvis-gpu-seed-"));
    const cacheBuild = path.join(tmpRoot, ".cache", "gpu-prebuild", "89", "llama", "build");
    const outBuild = path.join(tmpRoot, "target", "release", "build", "llama-sys", "out", "build");
    const outDir = path.join(outBuild, "..");
    const cudaLib = path.join(outBuild, "ggml", "src", "ggml-cuda", "ggml-cuda.lib");
    fs.mkdirSync(path.dirname(cudaLib), { recursive: true });
    fs.writeFileSync(cudaLib, "");
    const buildDirForward = path.resolve(outBuild).replace(/\\/g, "/");
    const outDirForward = path.resolve(outDir).replace(/\\/g, "/");
    const cacheWorkdir = `${path.resolve(cacheBuild).replace(/^([A-Za-z]):/, "$1$:").replace(/\//g, "\\")}\\`;
    fs.writeFileSync(
      path.join(outBuild, "CMakeCache.txt"),
      [
        `CMAKE_CACHEFILE_DIR:INTERNAL=${buildDirForward}`,
        `CMAKE_INSTALL_PREFIX:INTERNAL=${outDirForward}`,
        "BUILD_SHARED_LIBS:BOOL=OFF",
      ].join("\n"),
      "utf8",
    );
    fs.mkdirSync(outBuild, { recursive: true });
    fs.writeFileSync(
      path.join(outBuild, "build.ninja"),
      [
        `cmake_ninja_workdir = ${cacheWorkdir}`,
        `COMMAND = cd /D ${buildDirForward} && cmake --build`,
      ].join("\n"),
      "utf8",
    );
    expect(seededCmakeBuildNeedsRegen(outBuild)).toBe(true);
  });
});

describe("rewritePathPrefixesInSeededBuildTree", () => {
  /** @type {string | null} */
  let tmpRoot = null;

  afterEach(() => {
    if (tmpRoot && fs.existsSync(tmpRoot)) {
      fs.rmSync(tmpRoot, { recursive: true, force: true });
    }
    tmpRoot = null;
  });

  it("rewrites cmake_ninja_workdir C$: paths with trailing backslash", () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "jarvis-gpu-rewrite-"));
    const from = path.join(tmpRoot, ".cache", "gpu-prebuild", "89", "llama", "build");
    const to = path.join(tmpRoot, "target", "release", "build", "llama-out", "build");
    const buildDir = from;
    fs.mkdirSync(buildDir, { recursive: true });
    const fromWorkdir = `${path.resolve(from).replace(/^([A-Za-z]):/, "$1$:").replace(/\//g, "\\")}\\`;
    const toWorkdir = `${path.resolve(to).replace(/^([A-Za-z]):/, "$1$:").replace(/\//g, "\\")}\\`;
    fs.writeFileSync(
      path.join(buildDir, "build.ninja"),
      `cmake_ninja_workdir = ${fromWorkdir}\n`,
      "utf8",
    );

    rewritePathPrefixesInSeededBuildTree(buildDir, [[from, to]]);

    const ninja = fs.readFileSync(path.join(buildDir, "build.ninja"), "utf8");
    expect(ninja).toContain(toWorkdir);
    expect(ninja).not.toContain(`${path.sep}.cache${path.sep}gpu-prebuild${path.sep}`);
  });

  it("preserves path separators when rewriting build/cmake_install references", () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "jarvis-gpu-rewrite-"));
    const from = path.join(tmpRoot, ".cache", "gpu-prebuild", "89", "llama", "build");
    const to = path.join(tmpRoot, "target", "release", "build", "llama-out", "build");
    fs.mkdirSync(from, { recursive: true });
    const fromForward = path.resolve(from).replace(/\\/g, "/");
    const toForward = path.resolve(to).replace(/\\/g, "/");
    fs.writeFileSync(
      path.join(from, "build.ninja"),
      `COMMAND = cmake -P ${fromForward}/cmake_install.cmake\n`,
      "utf8",
    );

    rewritePathPrefixesInSeededBuildTree(from, [[from, to]]);

    const ninja = fs.readFileSync(path.join(from, "build.ninja"), "utf8");
    expect(ninja).toContain(`${toForward}/cmake_install.cmake`);
    expect(ninja).not.toContain("buildcmake_install");
  });
});

describe("llamaCudaPrebuildDefines", () => {
  it("forces static ggml/llama libs", () => {
    expect(llamaCudaPrebuildDefines().BUILD_SHARED_LIBS).toBe("OFF");
  });
});

describe("summarizeGpuPrebuildCache", () => {
  it("returns structured summary for diagnose script", () => {
    const summary = summarizeGpuPrebuildCache(JARVIS_ROOT, "89");
    expect(summary.arch).toBe("89");
    expect(summary.cacheRoot).toContain("gpu-prebuild");
    expect(summary.entries.whisper).toHaveProperty("warm");
    expect(summary.entries.llama).toHaveProperty("warm");
  });
});
