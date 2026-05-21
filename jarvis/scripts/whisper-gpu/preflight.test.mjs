import fs from "fs";
import os from "os";
import path from "path";
import { fileURLToPath } from "url";
import { afterEach, describe, expect, it } from "vitest";

import {
  cargoLockPaths,
  checkCargoBuildLock,
  isLikelyFirstCudaWhisperBuild,
} from "./preflight.mjs";

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
