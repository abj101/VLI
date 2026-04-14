/**
 * Downloads Porcupine DLL + .pv + .ppn from the public Picovoice GitHub repo (no API key).
 * Writes to src-tauri/resources/porcupine/. Gitignored; run via `npm run fetch-wake-models` or `prebuild`.
 */
import fs from "fs/promises";
import https from "https";
import path from "path";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const dest = path.join(__dirname, "..", "src-tauri", "resources", "porcupine");
const base = "https://raw.githubusercontent.com/Picovoice/porcupine/master";

const files = [
  ["lib/windows/amd64/libpv_porcupine.dll", "libpv_porcupine.dll"],
  ["lib/common/porcupine_params.pv", "porcupine_params.pv"],
  [
    "resources/keyword_files/windows/porcupine_windows.ppn",
    "porcupine_windows.ppn",
  ],
];

function download(url) {
  return new Promise((resolve, reject) => {
    https
      .get(url, (res) => {
        if (res.statusCode === 301 || res.statusCode === 302) {
          const loc = res.headers.location;
          if (!loc) {
            reject(new Error("Redirect without location"));
            return;
          }
          download(loc).then(resolve, reject);
          return;
        }
        if (res.statusCode !== 200) {
          reject(new Error(`GET ${url} -> ${res.statusCode}`));
          return;
        }
        const chunks = [];
        res.on("data", (c) => chunks.push(c));
        res.on("end", () => resolve(Buffer.concat(chunks)));
        res.on("error", reject);
      })
      .on("error", reject);
  });
}

await fs.mkdir(dest, { recursive: true });
for (const [rel, name] of files) {
  const out = path.join(dest, name);
  try {
    const st = await fs.stat(out);
    if (st.size > 0) {
      console.log(`fetch-wake-models: ${name} (skip, ${st.size} bytes)`);
      continue;
    }
  } catch {
    /* missing */
  }
  const url = `${base}/${rel}`;
  process.stdout.write(`fetch-wake-models: ${name} ... `);
  const buf = await download(url);
  await fs.writeFile(out, buf);
  console.log(`${buf.length} bytes`);
}
