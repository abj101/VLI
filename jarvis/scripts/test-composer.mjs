#!/usr/bin/env node
/**
 * Run composer fixture tests without cargo-win-env CMAKE generator conflicts.
 * Uses src-tauri/.cargo/config.local.toml (Ninja) — run `npm install` first.
 *
 * Usage:
 *   node scripts/test-composer.mjs        # mock fixture suite (CI)
 *   node scripts/test-composer.mjs --live # ignored live eval (needs llm-local + GGUF)
 */

import { spawnSync } from "child_process";
import path from "path";
import { fileURLToPath } from "url";

const JARVIS_ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");
const TAURI_ROOT = path.join(JARVIS_ROOT, "src-tauri");
const isLive = process.argv.includes("--live");

const env = { ...process.env };
for (const key of [
  "CMAKE_GENERATOR_INSTANCE",
  "CMAKE_GENERATOR_PLATFORM",
  "CMAKE_GENERATOR_TOOLSET",
]) {
  delete env[key];
}

const cargoArgs = isLive
  ? ["test", "composer_live_eval", "--", "--ignored", "--nocapture"]
  : ["test", "--no-default-features", "composer", "--", "--nocapture"];

const child = spawnSync("cargo", cargoArgs, {
  cwd: TAURI_ROOT,
  env,
  stdio: "inherit",
  shell: process.platform === "win32",
});

process.exit(child.status ?? 1);
