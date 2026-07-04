/**
 * Fetches bundled runtime models (wake + whisper + on-device LLM) before dev/build.
 *
 * Usage:
 *   node fetch-models.mjs
 *   node fetch-models.mjs --sets wake,whisper-tiny
 *   JARVIS_FETCH_SETS=wake,whisper-all,llm node fetch-models.mjs
 *
 * Set names:
 *   wake          — OpenWakeWord ONNX
 *   whisper-tiny  — ggml-tiny.en.bin only
 *   whisper-all   — tiny + base + small whisper weights
 *   llm           — router + composer GGUF
 */
import { spawn } from "child_process";
import path from "path";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const jarvisRoot = path.join(__dirname, "..");

const ALL_SETS = new Set(["wake", "whisper-tiny", "whisper-all", "llm"]);

/**
 * @returns {Set<string>}
 */
export function parseFetchSets(argv = process.argv.slice(2)) {
  const setsIdx = argv.indexOf("--sets");
  const fromArg = setsIdx >= 0 ? argv[setsIdx + 1]?.trim() : null;
  const fromEnv = process.env.JARVIS_FETCH_SETS?.trim();
  const raw = fromArg || fromEnv;
  if (!raw) {
    return new Set(ALL_SETS);
  }
  const sets = new Set(
    raw
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean),
  );
  for (const name of sets) {
    if (!ALL_SETS.has(name)) {
      throw new Error(
        `fetch-models: unknown set "${name}" (valid: ${[...ALL_SETS].join(", ")})`,
      );
    }
  }
  return sets;
}

function runNodeScript(name, extraArgs = []) {
  return new Promise((resolve, reject) => {
    const script = path.join(__dirname, name);
    const child = spawn(process.execPath, [script, ...extraArgs], {
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

/**
 * @param {string[]} modelSets from resolveDevProfile (wake, whisper-tiny, whisper-all, llm)
 */
export async function fetchModelsForSets(modelSets) {
  const sets = new Set(modelSets);
  if (sets.has("whisper-all")) {
    await runNodeScript("fetch-whisper-models.mjs");
  } else if (sets.has("whisper-tiny")) {
    await runNodeScript("fetch-whisper-models.mjs", ["--only", "tiny"]);
  }
  if (sets.has("wake")) {
    await runNodeScript("fetch-wake-models.mjs");
  }
  if (sets.has("llm")) {
    await runNodeScript("fetch-llm-models.mjs");
  }
}

const isMain = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);

if (isMain) {
  const sets = parseFetchSets();
  await fetchModelsForSets([...sets]);
}
