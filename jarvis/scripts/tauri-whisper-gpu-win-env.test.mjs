import { describe, expect, it } from "vitest";

import {
  buildWindowsWhisperCargoEnv,
  discoverWindowsMsvcIncludeDir,
  tomlEnvValue,
  validateWindowsWhisperBindgenEnv,
  windowsBindgenExtraClangArgs,
  windowsShortPath,
} from "./whisper-gpu/win-env.mjs";

describe("buildWindowsWhisperCargoEnv", () => {
  it("pins VS 2022 CMake generator when unset", () => {
    if (process.platform !== "win32") return;
    const env = buildWindowsWhisperCargoEnv({});
    expect(env.CMAKE_GENERATOR).toBe("Visual Studio 17 2022");
  });

  it("omits CMAKE_GENERATOR when includeCmakeGenerator is false (sync path)", () => {
    if (process.platform !== "win32") return;
    const env = buildWindowsWhisperCargoEnv({}, { includeCmakeGenerator: false });
    expect(env.CMAKE_GENERATOR).toBeUndefined();
  });

  it("does not override CMAKE_GENERATOR when already set", () => {
    if (process.platform !== "win32") return;
    const env = buildWindowsWhisperCargoEnv({ CMAKE_GENERATOR: "NMake Makefiles" });
    expect(env.CMAKE_GENERATOR).toBeUndefined();
  });
});

describe("discoverWindowsMsvcIncludeDir", () => {
  it("resolves include three levels above Hostx64/x64", () => {
    if (process.platform !== "win32") return;
    const inc = discoverWindowsMsvcIncludeDir();
    if (!inc) return;
    expect(inc.replace(/\\/g, "/")).toMatch(/\/MSVC\/[^/]+\/include$/);
  });
});

describe("tomlEnvValue", () => {
  it("uses a single-line basic string (no multiline leading newline)", () => {
    const toml = tomlEnvValue('-isystem "C:/foo bar/include"');
    expect(toml).toBe(JSON.stringify('-isystem "C:/foo bar/include"'));
    expect(toml).not.toMatch(/^"""/);
  });
});

describe("windowsBindgenExtraClangArgs", () => {
  it("includes -std=c11 when discovery succeeds", () => {
    if (process.platform !== "win32") return;
    const args = windowsBindgenExtraClangArgs();
    if (!args) return;
    expect(args).toContain("-std=c11");
  });

  it("uses -isystem and no leading whitespace (bindgen shlex + Cargo [env])", () => {
    if (process.platform !== "win32") return;
    const args = windowsBindgenExtraClangArgs();
    if (!args) return;
    expect(args).toContain("-isystem");
    expect(args.trim()).toBe(args);
    expect(args.startsWith("\n")).toBe(false);
  });
});

describe("windowsShortPath", () => {
  it("returns forward slashes", () => {
    if (process.platform !== "win32") return;
    const p = windowsShortPath(process.env.SystemRoot ?? "C:\\Windows");
    expect(p).toMatch(/^\w:\//);
  });
});

describe("buildWindowsWhisperCargoEnv force", () => {
  it("overwrites stale BINDGEN when force is true", () => {
    if (process.platform !== "win32") return;
    const fresh = windowsBindgenExtraClangArgs();
    if (!fresh) return;
    const env = buildWindowsWhisperCargoEnv(
      { BINDGEN_EXTRA_CLANG_ARGS: "-broken" },
      { force: true },
    );
    expect(env.BINDGEN_EXTRA_CLANG_ARGS).toBe(fresh);
  });
});

describe("validateWindowsWhisperBindgenEnv", () => {
  it("returns structured errors on Windows when discovery fails", () => {
    if (process.platform !== "win32") return;
    const check = validateWindowsWhisperBindgenEnv();
    if (check.ok) {
      expect(check.errors).toHaveLength(0);
      expect(windowsBindgenExtraClangArgs()).toBeTruthy();
      return;
    }
    expect(check.errors.length).toBeGreaterThan(0);
    expect(check.warnings.some((w) => w.includes("clean -p whisper-rs-sys"))).toBe(true);
  });
});
