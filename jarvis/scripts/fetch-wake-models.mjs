/**
 * Downloads OpenWakeWord ONNX assets (no API key). Writes under
 * `src-tauri/resources/oww/`. Gitignored; run via `npm run fetch-wake-models`,
 * `prebuild`, or Tauri `beforeDevCommand`.
 */
import fs from "fs/promises";
import https from "https";
import path from "path";
import { fileURLToPath } from "url";
import { oww } from "./fetch-wake-models.config.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const resourcesRoot = path.join(__dirname, "..", "src-tauri", "resources");

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

async function fetchIfMissing(label, destDir, url, outName) {
  const out = path.join(destDir, outName);
  try {
    const st = await fs.stat(out);
    if (st.size > 0) {
      console.log(`fetch-wake-models: ${label}${outName} (skip, ${st.size} bytes)`);
      return;
    }
  } catch {
    /* missing */
  }
  process.stdout.write(`fetch-wake-models: ${label}${outName} ... `);
  const buf = await download(url);
  await fs.writeFile(out, buf);
  console.log(`${buf.length} bytes`);
}

await fs.mkdir(resourcesRoot, { recursive: true });

const owwDest = path.join(resourcesRoot, oww.destSubdir);
await fs.mkdir(owwDest, { recursive: true });
for (const name of oww.files) {
  const url = `${oww.base}/${name}`;
  await fetchIfMissing("oww/", owwDest, url, name);
}
