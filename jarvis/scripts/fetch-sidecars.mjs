import { spawnSync } from "node:child_process";
import { createWriteStream } from "node:fs";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";
import { copyFile, cp, mkdir, mkdtemp, readFile, readdir, rm, stat, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const binDir = path.join(root, "src-tauri", "resources", "bin");
const whisperTag = "b5454";
const llamaTag = "b11998";

async function download(url, file) {
  const response = await fetch(url);
  if (!response.ok || !response.body) throw new Error(`Download failed (${response.status}): ${url}`);
  await pipeline(Readable.fromWeb(response.body), createWriteStream(file));
}

function expand(zip, destination) {
  const result = spawnSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", "Expand-Archive -LiteralPath $env:JARVIS_SIDECAR_ZIP -DestinationPath $env:JARVIS_SIDECAR_DEST -Force"], {
    stdio: "inherit",
    env: { ...process.env, JARVIS_SIDECAR_ZIP: zip, JARVIS_SIDECAR_DEST: destination },
  });
  if (result.error || result.status !== 0) throw result.error ?? new Error(`Expand-Archive failed (${result.status})`);
}

async function filesUnder(dir) {
  const files = [];
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) files.push(...await filesUnder(file));
    else files.push(file);
  }
  return files;
}

async function extractRuntime(zip, destination, executable) {
  const extracted = `${destination}-extracted`;
  await mkdir(extracted, { recursive: true });
  expand(zip, extracted);
  const files = await filesUnder(extracted);
  const exe = files.find((file) => path.basename(file).toLowerCase() === executable.toLowerCase());
  if (!exe) throw new Error(`${executable} was not found in ${path.basename(zip)}`);
  await mkdir(destination, { recursive: true });
  for (const file of files) {
    if (file === exe || path.extname(file).toLowerCase() === ".dll") {
      await copyFile(file, path.join(destination, path.basename(file)));
    }
  }
  await rm(extracted, { recursive: true, force: true });
}

if (process.platform === "darwin") {
  for (const [exe, args] of [["whisper-server", ["--help"]], ["llama-server", ["--version"]]]) {
    const result = spawnSync(exe, args, { stdio: "ignore" });
    if (result.error?.code === "ENOENT") {
      console.error(`Missing ${exe} on PATH. Install the macOS sidecars with: brew install whisper.cpp llama.cpp`);
      process.exitCode = 1;
      break;
    }
    if (result.error || result.status !== 0) throw result.error ?? new Error(`${exe} exited with code ${result.status}`);
  }
  await mkdir(binDir, { recursive: true });
  await writeFile(path.join(binDir, ".sidecars-path"), "PATH-managed\n");
} else if (process.platform !== "win32") {
  throw new Error("Prebuilt Jarvis sidecars are currently configured for Windows; install whisper.cpp and llama.cpp on this platform.");
} else {
  const cuda = spawnSync("nvidia-smi", ["-L"], { stdio: "ignore" }).status === 0;
  const whisperAsset = cuda ? "whisper-bin-win-cuda-12.4.0-x64.zip" : "whisper-bin-x64.zip";
  const llamaAsset = cuda ? "llama-b11998-bin-win-cuda-12.4-x64.zip" : "llama-b11998-bin-win-cpu-x64.zip";
  const marker = JSON.stringify({ whisperTag, whisperAsset, llamaTag, llamaAsset, cudart: cuda ? "cudart-llama-bin-win-cuda-12.4-x64.zip" : null });
  const markerFile = path.join(binDir, ".sidecars-version.json");
  try {
    const current = await readFile(markerFile, "utf8");
    await Promise.all(["whisper/whisper-server.exe", "llama/llama-server.exe"].map((file) => stat(path.join(binDir, file))));
    if (current === marker) {
      console.log(`Sidecars already fetched (${cuda ? "CUDA 12.4" : "CPU"}).`);
      process.exit(0);
    }
  } catch {}

  const temp = await mkdtemp(path.join(os.tmpdir(), "jarvis-sidecars-"));
  try {
    const whisperZip = path.join(temp, whisperAsset);
    const llamaZip = path.join(temp, llamaAsset);
    await download(`https://github.com/ggml-org/whisper.cpp/releases/download/${whisperTag}/${whisperAsset}`, whisperZip);
    await download(`https://github.com/ggml-org/llama.cpp/releases/download/${llamaTag}/${llamaAsset}`, llamaZip);
    const staged = path.join(temp, "bin");
    await extractRuntime(whisperZip, path.join(staged, "whisper"), "whisper-server.exe");
    await extractRuntime(llamaZip, path.join(staged, "llama"), "llama-server.exe");
    if (cuda) {
      const cudartAsset = "cudart-llama-bin-win-cuda-12.4-x64.zip";
      const cudartZip = path.join(temp, cudartAsset);
      await download(`https://github.com/ggml-org/llama.cpp/releases/download/${llamaTag}/${cudartAsset}`, cudartZip);
      const cudartExtracted = path.join(temp, "cudart-extracted");
      await mkdir(cudartExtracted, { recursive: true });
      expand(cudartZip, cudartExtracted);
      for (const file of await filesUnder(cudartExtracted)) {
        if (path.extname(file).toLowerCase() === ".dll") {
          await copyFile(file, path.join(staged, "llama", path.basename(file)));
        }
      }
    }
    await mkdir(binDir, { recursive: true });
    for (const runtime of ["whisper", "llama"]) {
      const target = path.join(binDir, runtime);
      await rm(target, { recursive: true, force: true });
      await cp(path.join(staged, runtime), target, { recursive: true });
    }
    await writeFile(markerFile, marker);
    console.log(`Fetched ${whisperTag} and ${llamaTag} sidecars (${cuda ? "CUDA 12.4" : "CPU"}).`);
  } finally {
    await rm(temp, { recursive: true, force: true });
  }
}
