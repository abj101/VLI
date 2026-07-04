/**
 * Downloads bundled Whisper ggml weights into src-tauri/resources (skip existing files).
 *
 * Usage:
 *   node fetch-whisper-models.mjs              # all models
 *   node fetch-whisper-models.mjs --only tiny  # ggml-tiny.en.bin only
 *   JARVIS_WHISPER_ONLY=tiny node fetch-whisper-models.mjs
 */
import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const destDir = path.join(__dirname, "..", "src-tauri", "resources");
const BASE = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

const MODELS = [
  { file: "ggml-tiny.en.bin", tier: "tiny" },
  { file: "ggml-base.en.bin", tier: "base" },
  { file: "ggml-small.en.bin", tier: "small" },
];

/**
 * @returns {Set<string> | null} null = fetch all tiers
 */
function resolveWhisperTiers() {
  const argv = process.argv.slice(2);
  const onlyIdx = argv.indexOf("--only");
  const fromArg = onlyIdx >= 0 ? argv[onlyIdx + 1]?.trim() : null;
  const fromEnv = process.env.JARVIS_WHISPER_ONLY?.trim();
  const only = fromArg || fromEnv;
  if (!only) return null;
  return new Set(
    only
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean),
  );
}

async function download(url, dest) {
  const res = await fetch(url);
  if (!res.ok) {
    throw new Error(`HTTP ${res.status} for ${url}`);
  }
  const buf = Buffer.from(await res.arrayBuffer());
  fs.writeFileSync(dest, buf);
}

const tiers = resolveWhisperTiers();
const models =
  tiers === null ? MODELS : MODELS.filter((m) => tiers.has(m.tier));

if (models.length === 0) {
  console.error(
    "fetch-whisper-models: no models matched filter (tiers: tiny, base, small)",
  );
  process.exit(1);
}

fs.mkdirSync(destDir, { recursive: true });

for (const { file, tier } of models) {
  const dest = path.join(destDir, file);
  if (fs.existsSync(dest)) {
    console.log(`fetch-whisper-models: skip ${file} (already on disk)`);
    continue;
  }
  const url = `${BASE}/${file}`;
  console.log(`fetch-whisper-models: downloading ${file} (${tier})…`);
  await download(url, dest);
  console.log(`fetch-whisper-models: wrote ${dest}`);
}

console.log("fetch-whisper-models: done");
