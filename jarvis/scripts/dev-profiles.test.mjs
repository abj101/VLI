import path from "path";
import { fileURLToPath } from "url";
import { describe, expect, it } from "vitest";

import {
  parseDevProfileFlags,
  resolveDevProfile,
  validatePlatformHost,
} from "./dev-profiles.mjs";

const JARVIS_ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");

describe("parseDevProfileFlags", () => {
  it("strips --mac, --win, --audio from argv", () => {
    const { extraArgs, platformFlag, scopeFlag } = parseDevProfileFlags([
      "dev",
      "--mac",
      "--audio",
      "--release",
    ]);
    expect(platformFlag).toBe("mac");
    expect(scopeFlag).toBe("audio");
    expect(extraArgs).toEqual(["dev", "--release"]);
  });

  it("rejects both --mac and --win", () => {
    expect(() => parseDevProfileFlags(["dev", "--mac", "--win"])).toThrow(
      /only one of --mac or --win/,
    );
  });
});

describe("validatePlatformHost", () => {
  it("auto always passes", () => {
    expect(validatePlatformHost("auto", "darwin").ok).toBe(true);
    expect(validatePlatformHost("auto", "win32").ok).toBe(true);
  });

  it("mac requires darwin", () => {
    expect(validatePlatformHost("mac", "darwin").ok).toBe(true);
    expect(validatePlatformHost("mac", "win32").ok).toBe(false);
  });

  it("win requires win32", () => {
    expect(validatePlatformHost("win", "win32").ok).toBe(true);
    expect(validatePlatformHost("win", "darwin").ok).toBe(false);
  });
});

describe("resolveDevProfile", () => {
  it("default auto/full/gpu uses null targetDir", () => {
    const p = resolveDevProfile({
      platformFlag: "auto",
      scopeFlag: "full",
      hostPlatform: "darwin",
      backend: "metal",
      jarvisRoot: JARVIS_ROOT,
    });
    expect(p.usesDefaultTargetDir).toBe(true);
    expect(p.targetDir).toBeNull();
    expect(p.cargoFeatures).toEqual(["oww", "llm-local", "whisper-metal", "llm-metal"]);
    expect(p.modelSets).toEqual(["wake", "whisper-all", "llm"]);
  });

  it("audio scope omits llm features and models", () => {
    const p = resolveDevProfile({
      platformFlag: "mac",
      scopeFlag: "audio",
      hostPlatform: "darwin",
      backend: "metal",
      jarvisRoot: JARVIS_ROOT,
    });
    expect(p.cargoFeatures).toEqual(["oww", "whisper-metal"]);
    expect(p.noDefaultFeatures).toBe(true);
    expect(p.modelSets).toEqual(["wake", "whisper-tiny"]);
    expect(p.targetDir).toBe(
      path.join(JARVIS_ROOT, ".cache", "cargo-target", "mac-audio-metal"),
    );
  });

  it("cpu full uses isolated target dir", () => {
    const p = resolveDevProfile({
      platformFlag: "auto",
      scopeFlag: "full",
      hostPlatform: "win32",
      backend: "none",
      jarvisRoot: JARVIS_ROOT,
    });
    expect(p.cargoFeatures).toEqual(["oww", "llm-local"]);
    expect(p.targetDir).toBe(
      path.join(JARVIS_ROOT, ".cache", "cargo-target", "win-cpu-full"),
    );
  });

  it("explicit --win full cuda gets isolated target", () => {
    const p = resolveDevProfile({
      platformFlag: "win",
      scopeFlag: "full",
      hostPlatform: "win32",
      backend: "cuda",
      jarvisRoot: JARVIS_ROOT,
    });
    expect(p.targetDir).toBe(
      path.join(JARVIS_ROOT, ".cache", "cargo-target", "win-full-cuda"),
    );
    expect(p.logLabel).toContain("explicit --win");
  });

  it("throws on platform mismatch", () => {
    expect(() =>
      resolveDevProfile({
        platformFlag: "mac",
        scopeFlag: "full",
        hostPlatform: "win32",
        backend: "metal",
        jarvisRoot: JARVIS_ROOT,
      }),
    ).toThrow(/requires host OS darwin/);
  });
});
