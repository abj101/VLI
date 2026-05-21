#!/usr/bin/env node
/**
 * Run `cargo` with Windows whisper-rs-sys env (sync-cargo-win-env / whisper-gpu).
 * Usage: node scripts/cargo-win-env.mjs check --manifest-path src-tauri/Cargo.toml --features oww
 */

import { spawnSync } from "child_process";
import path from "path";
import { fileURLToPath } from "url";

import {
  applyDiscoveredCudaToolkitToProcessEnv,
  applyWindowsCudaBuildEnvIfNeeded,
} from "./whisper-gpu/detect.mjs";
import {
  assertWindowsWhisperBindgenEnv,
  buildWindowsWhisperCargoEnv,
} from "./whisper-gpu/win-env.mjs";
import { checkCargoBuildLock } from "./whisper-gpu/preflight.mjs";

const JARVIS_ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
if (args.length === 0) {
  console.error(
    "cargo-win-env: pass cargo subcommand and args, e.g. check --manifest-path src-tauri/Cargo.toml",
  );
  process.exit(1);
}

/** True when cargo args request whisper-cuda (must use NMake + nvcc on Windows). */
function cargoArgsNeedCudaEnv(cargoArgs) {
  const joined = cargoArgs.join(" ").toLowerCase();
  return joined.includes("whisper-cuda");
}

if (process.platform === "win32") {
  const lock = checkCargoBuildLock(JARVIS_ROOT);
  if (lock.blocked) {
    console.error(`cargo-win-env: ${lock.message}`);
    process.exit(1);
  }
  assertWindowsWhisperBindgenEnv("cargo-win-env");
}

const needsCuda = process.platform === "win32" && cargoArgsNeedCudaEnv(args);

/** @type {NodeJS.ProcessEnv} */
let env = { ...process.env };

if (process.platform === "win32") {
  env = {
    ...env,
    ...buildWindowsWhisperCargoEnv(env, {
      force: true,
      includeCmakeGenerator: !needsCuda,
    }),
    CARGO_TERM_PROGRESS: env.CARGO_TERM_PROGRESS ?? "always",
  };
  if (needsCuda) {
    applyDiscoveredCudaToolkitToProcessEnv();
    const cudaBuildEnv = applyWindowsCudaBuildEnvIfNeeded(env);
    if (!cudaBuildEnv) {
      console.error(
        "cargo-win-env: whisper-cuda requires CUDA toolkit + MSVC (NMake). Use `npm run tauri dev` or install CUDA.",
      );
      process.exit(1);
    }
    console.log(
      `cargo-win-env: CUDA build env (generator=${cudaBuildEnv.generator}; CUDA_PATH=${cudaBuildEnv.cudaRoot})`,
    );
  }
}

const child = spawnSync("cargo", args, {
  cwd: JARVIS_ROOT,
  env,
  stdio: "inherit",
  shell: process.platform === "win32",
});

process.exit(child.status ?? 1);
