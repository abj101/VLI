/**
 * Fetches all bundled runtime models (wake + on-device LLM) before dev/build.
 */
import { spawn } from "child_process";
import path from "path";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const jarvisRoot = path.join(__dirname, "..");

function runNodeScript(name) {
  return new Promise((resolve, reject) => {
    const script = path.join(__dirname, name);
    const child = spawn(process.execPath, [script], {
      cwd: jarvisRoot,
      stdio: "inherit",
      env: process.env,
    });
    child.on("error", reject);
    child.on("exit", (code) => {
      if (code === 0) resolve();
      else reject(new Error(`${name} exited with code ${code}`));
    });
  });
}

await runNodeScript("fetch-whisper-models.mjs");
await runNodeScript("fetch-wake-models.mjs");
await runNodeScript("fetch-llm-models.mjs");
