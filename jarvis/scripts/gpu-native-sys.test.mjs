import fs from "fs";
import os from "os";
import path from "path";
import { afterEach, describe, expect, it } from "vitest";

import {
  GPU_NATIVE_BUILD_PREFIXES,
  clearUnusableGpuSysCmakeCaches,
  isCmakeCacheWellFormed,
  readCmakeCacheValue,
  readCachedCmakeGenerator,
  repairCmakeCacheMalformedEntries,
  resolveBestGpuSysOutBuildDir,
} from "./gpu-native-sys.mjs";
import { rewritePathPrefixesInSeededBuildTree } from "./gpu-prebuild.mjs";

describe("GPU_NATIVE_BUILD_PREFIXES", () => {
  it("lists whisper-rs-sys and llama-cpp-sys-2 cargo build dirs", () => {
    expect(GPU_NATIVE_BUILD_PREFIXES.map((p) => p.prefix)).toEqual([
      "whisper-rs-sys-",
      "llama-cpp-sys-2-",
    ]);
  });
});

describe("resolveBestGpuSysOutBuildDir", () => {
  /** @type {string | null} */
  let tmpRoot = null;

  afterEach(() => {
    if (tmpRoot) {
      fs.rmSync(tmpRoot, { recursive: true, force: true });
      tmpRoot = null;
    }
  });

  it("picks the hashed dir that actually built ggml-cuda", () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "jarvis-gpu-native-dup-"));
    const buildRoot = path.join(tmpRoot, "build");

    fs.mkdirSync(path.join(buildRoot, "whisper-rs-sys-aaa", "out", "build"), {
      recursive: true,
    });

    const warmOut = path.join(buildRoot, "whisper-rs-sys-zzz", "out", "build");
    const warmLib = path.join(warmOut, "ggml", "src", "ggml-cuda", "ggml-cuda.lib");
    fs.mkdirSync(path.dirname(warmLib), { recursive: true });
    fs.writeFileSync(warmLib, "");

    expect(resolveBestGpuSysOutBuildDir(buildRoot, "whisper-rs-sys-")).toBe(warmOut);
  });
});

describe("readCmakeCacheValue", () => {
  it("reads INTERNAL, PATH, STRING, and BOOL entries", () => {
    const text = [
      "CMAKE_HOME_DIRECTORY:INTERNAL=C:/src",
      "CMAKE_INSTALL_PREFIX:PATH=C:/out",
      "BUILD_SHARED_LIBS:BOOL=OFF",
      "CMAKE_GENERATOR:STRING=Ninja",
    ].join("\n");
    expect(readCmakeCacheValue(text, "CMAKE_HOME_DIRECTORY")).toBe("C:/src");
    expect(readCmakeCacheValue(text, "CMAKE_INSTALL_PREFIX")).toBe("C:/out");
    expect(readCmakeCacheValue(text, "BUILD_SHARED_LIBS")).toBe("OFF");
    expect(readCmakeCacheValue(text, "CMAKE_GENERATOR")).toBe("Ninja");
    expect(readCmakeCacheValue(text, "MISSING")).toBeNull();
  });
});

describe("readCachedCmakeGenerator", () => {
  it("prefers CMAKE_GENERATOR cache entries", () => {
    const text = "CMAKE_GENERATOR:INTERNAL=Ninja\nCMAKE_MAKE_PROGRAM:FILEPATH=ninja.exe";
    expect(readCachedCmakeGenerator(text)).toBe("Ninja");
  });
});

describe("repairCmakeCacheMalformedEntries", () => {
  it("fixes bare KEY=VALUE lines left by legacy path rewrite on CMakeCache.txt", () => {
    const text = [
      "CMAKE_GENERATOR:INTERNAL=Ninja",
      "",
      "CMAKE_BUILD_TYPE=Release",
    ].join("\n");
    const repaired = repairCmakeCacheMalformedEntries(text);
    expect(isCmakeCacheWellFormed(repaired)).toBe(true);
    expect(repaired).toContain("CMAKE_BUILD_TYPE:STRING=Release");
    expect(readCmakeCacheValue(repaired, "CMAKE_BUILD_TYPE")).toBe("Release");
  });
});

describe("isCmakeCacheWellFormed", () => {
  it("accepts a typical CMake cache file", () => {
    const text = [
      "# This is the CMakeCache file.",
      "CMAKE_BUILD_TYPE:STRING=Release",
      "CMAKE_GENERATOR:INTERNAL=Ninja",
      "",
    ].join("\n");
    expect(isCmakeCacheWellFormed(text)).toBe(true);
  });

  it("rejects entries missing the TYPE colon (CMake parse error shape)", () => {
    const text = [
      "CMAKE_GENERATOR:INTERNAL=Ninja",
      "",
      "CMAKE_BUILD_TYPE=Release",
    ].join("\n");
    expect(isCmakeCacheWellFormed(text)).toBe(false);
  });

  it("allows empty CMake cache values (KEY:TYPE=)", () => {
    const text = [
      "CMAKE_GENERATOR:INTERNAL=Ninja",
      "CMAKE_ASM_FLAGS:STRING=",
      "CMAKE_BUILD_TYPE:STRING=Release",
    ].join("\n");
    expect(isCmakeCacheWellFormed(text)).toBe(true);
  });

  it("rejects data lines without KEY:TYPE= format", () => {
    const text = "not-a-cache-line\nCMAKE_BUILD_TYPE:STRING=Release";
    expect(isCmakeCacheWellFormed(text)).toBe(false);
  });
});

describe("clearUnusableGpuSysCmakeCaches", () => {
  /** @type {string | null} */
  let tmpRoot = null;

  afterEach(() => {
    if (tmpRoot) {
      fs.rmSync(tmpRoot, { recursive: true, force: true });
      tmpRoot = null;
    }
  });

  it("removes GPU -sys trees with corrupt CMakeCache before cargo configure", () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "jarvis-gpu-native-sys-"));
    const outBuild = path.join(
      tmpRoot,
      "src-tauri",
      "target",
      "release",
      "build",
      "llama-cpp-sys-2-deadbeef",
      "out",
      "build",
    );
    fs.mkdirSync(outBuild, { recursive: true });
    fs.writeFileSync(
      path.join(outBuild, "CMakeCache.txt"),
      ["CMAKE_GENERATOR:INTERNAL=Ninja", "", "CMAKE_BUILD_TYPE=Release"].join("\n"),
      "utf8",
    );
    fs.writeFileSync(path.join(outBuild, "build.ninja"), "# stale\n", "utf8");

    const { cleared } = clearUnusableGpuSysCmakeCaches(tmpRoot, "release");
    expect(cleared).toHaveLength(1);
    expect(fs.existsSync(outBuild)).toBe(false);
  });

  it("removes incomplete trees (CMakeCache without build.ninja)", () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "jarvis-gpu-native-sys-"));
    const outBuild = path.join(
      tmpRoot,
      "src-tauri",
      "target",
      "debug",
      "build",
      "whisper-rs-sys-cafebabe",
      "out",
      "build",
    );
    fs.mkdirSync(outBuild, { recursive: true });
    fs.writeFileSync(
      path.join(outBuild, "CMakeCache.txt"),
      "CMAKE_GENERATOR:INTERNAL=Ninja\n",
      "utf8",
    );

    const { cleared } = clearUnusableGpuSysCmakeCaches(tmpRoot, "debug");
    expect(cleared).toHaveLength(1);
    expect(fs.existsSync(outBuild)).toBe(false);
  });
});

describe("rewritePathPrefixesInSeededBuildTree", () => {
  /** @type {string | null} */
  let tmpRoot = null;

  afterEach(() => {
    if (tmpRoot) {
      fs.rmSync(tmpRoot, { recursive: true, force: true });
      tmpRoot = null;
    }
  });

  it("does not rewrite CMakeCache.txt (paths fixed via rewriteSeededCmakeCache)", () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "jarvis-gpu-native-sys-"));
    const buildDir = path.join(tmpRoot, "build");
    fs.mkdirSync(buildDir, { recursive: true });
    const cachePath = path.join(buildDir, "CMakeCache.txt");
    const from = "C:/old/cache/build";
    const to = path.join(tmpRoot, "out", "build").replace(/\\/g, "/");
    fs.writeFileSync(
      cachePath,
      `CMAKE_CACHEFILE_DIR:INTERNAL=${from}\nCMAKE_GENERATOR:INTERNAL=Ninja\n`,
      "utf8",
    );
    fs.writeFileSync(path.join(buildDir, "build.ninja"), `builddir = ${from}\n`, "utf8");

    rewritePathPrefixesInSeededBuildTree(buildDir, [[from, to]]);

    const cacheText = fs.readFileSync(cachePath, "utf8");
    expect(cacheText).toContain(from);
    expect(isCmakeCacheWellFormed(cacheText)).toBe(true);
    const ninjaText = fs.readFileSync(path.join(buildDir, "build.ninja"), "utf8");
    expect(ninjaText).toContain(to);
  });
});
