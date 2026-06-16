#!/usr/bin/env node
/**
 * Run `cargo` with Windows whisper-rs-sys env (sync-cargo-win-env / whisper-gpu).
 * Usage: node scripts/cargo-win-env.mjs check --manifest-path src-tauri/Cargo.toml --features oww
 */

import { spawnSync } from "child_process";
import path from "path";
import { fileURLToPath } from "url";

import {
  formatResolvedCudaLog,
  prepareGpuNativeBuild,
  resolveBuildEnvironment,
} from "./build-environment.mjs";
import { checkCargoBuildLock } from "./whisper-gpu/preflight.mjs";

const JARVIS_ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
if (args.length === 0) {
  console.error(
    "cargo-win-env: pass cargo subcommand and args, e.g. check --manifest-path src-tauri/Cargo.toml",
  );
  process.exit(1);
}

/** True when cargo args request CUDA features (must use NMake/Ninja + nvcc on Windows). */
function cargoArgsNeedCudaEnv(cargoArgs) {
  const joined = cargoArgs.join(" ").toLowerCase();
  return joined.includes("whisper-cuda") || joined.includes("llm-cuda");
}

if (process.platform === "win32") {
  const lock = checkCargoBuildLock(JARVIS_ROOT);
  if (lock.blocked) {
    console.error(`cargo-win-env: ${lock.message}`);
    process.exit(1);
  }
}

const needsCuda = process.platform === "win32" && cargoArgsNeedCudaEnv(args);

const { env, cudaBuildEnv, generator, gpuPrebuild } = resolveBuildEnvironment({
  jarvisRoot: JARVIS_ROOT,
  channel: "cargo",
  baseEnv: process.env,
  needsCuda,
  logPrefix: "cargo-win-env",
});

if (needsCuda && !cudaBuildEnv) {
  console.error(
    "cargo-win-env: whisper-cuda / llm-cuda requires CUDA toolkit + MSVC (NMake/Ninja). Use `npm run tauri dev` or install CUDA.",
  );
  process.exit(1);
}

if (cudaBuildEnv) {
  const cudaLog = formatResolvedCudaLog(cudaBuildEnv, gpuPrebuild);
  if (cudaLog) console.log(`cargo-win-env: ${cudaLog}`);
  prepareGpuNativeBuild(JARVIS_ROOT, env, {
    logPrefix: "cargo-win-env",
    profile: "debug",
    generator: generator ?? undefined,
    warnGeneratorMismatch: false,
  });
}

const child = spawnSync("cargo", args, {
  cwd: JARVIS_ROOT,
  env,
  stdio: "inherit",
  shell: process.platform === "win32",
});

process.exit(child.status ?? 1);
