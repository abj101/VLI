#!/usr/bin/env node
/**
 * npm precargo hook — Windows bindgen sync + optional model fetch.
 * Set JARVIS_SKIP_MODEL_FETCH=1 for fast bare cargo check.
 */
import { spawnSync } from "child_process";
import path from "path";
import { fileURLToPath } from "url";

const JARVIS_ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");
const scripts = path.join(JARVIS_ROOT, "scripts");

if (process.platform === "win32") {
  const r = spawnSync(process.execPath, [path.join(scripts, "sync-cargo-win-env.mjs")], {
    cwd: JARVIS_ROOT,
    stdio: "inherit",
  });
  if (r.status !== 0) {
    process.exit(r.status ?? 1);
  }
}

if (!process.env.JARVIS_SKIP_MODEL_FETCH?.trim()) {
  const r = spawnSync(process.execPath, [path.join(scripts, "fetch-models.mjs")], {
    cwd: JARVIS_ROOT,
    stdio: "inherit",
  });
  if (r.status !== 0) {
    process.exit(r.status ?? 1);
  }
}
