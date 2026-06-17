/**
 * Downloads bundled Whisper ggml weights into src-tauri/resources (skip existing files).
 */
import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const destDir = path.join(__dirname, "..", "src-tauri", "resources");
const BASE = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

const MODELS = [
  { file: "ggml-tiny.en.bin" },
  { file: "ggml-base.en.bin" },
  { file: "ggml-small.en.bin" },
];

async function download(url, dest) {
  const res = await fetch(url);
  if (!res.ok) {
    throw new Error(`HTTP ${res.status} for ${url}`);
  }
  const buf = Buffer.from(await res.arrayBuffer());
  fs.writeFileSync(dest, buf);
}

fs.mkdirSync(destDir, { recursive: true });

for (const { file } of MODELS) {
  const dest = path.join(destDir, file);
  if (fs.existsSync(dest)) {
    console.log(`fetch-whisper-models: skip ${file} (already on disk)`);
    continue;
  }
  const url = `${BASE}/${file}`;
  console.log(`fetch-whisper-models: downloading ${file}…`);
  await download(url, dest);
  console.log(`fetch-whisper-models: wrote ${dest}`);
}

console.log("fetch-whisper-models: done");
