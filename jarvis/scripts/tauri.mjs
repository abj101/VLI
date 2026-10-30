// Thin `tauri` launcher: fetch models, pick GPU features, run inside the MSVC dev env on Windows.
// Usage: node scripts/tauri.mjs dev|build [--cpu] [tauri args]
import { execFileSync, execSync, spawnSync } from "child_process";
import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const cpu = args.includes("--cpu") || args.includes("--no-gpu");
const rest = args.filter((a) => a !== "--cpu" && a !== "--no-gpu");
const env = { ...process.env };

if (process.platform === "win32") {
  // Ninja + cl.exe + nvcc need the vcvars env; one fixed generator keeps the CMake caches valid.
  const vswhere = path.join(process.env["ProgramFiles(x86)"], "Microsoft Visual Studio", "Installer", "vswhere.exe");
  const vs = execFileSync(vswhere, ["-latest", "-products", "*", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath"], { encoding: "utf8" }).trim();
  const dump = execSync(`"${path.join(vs, "VC", "Auxiliary", "Build", "vcvars64.bat")}" >nul && set`, { encoding: "utf8", maxBuffer: 1 << 24 });
  for (const line of dump.split(/\r?\n/)) {
    const i = line.indexOf("=");
    if (i > 0) env[line.slice(0, i)] = line.slice(i + 1);
  }
  env.CMAKE_GENERATOR = "Ninja";
  env.CCACHE_DISABLE = "1"; // ccache + MSVC yields empty .obj (LNK1181)
  env.CL = env._CL_ = "/Zc:preprocessor"; // CUDA 13 CCCL needs the conforming preprocessor; cl.exe reads CL itself, even under nvcc
}

const features = !cpu && process.platform !== "darwin" ? ["whisper-cuda", "llm-cuda"] : [];
const cmd = [path.join(root, "node_modules", "@tauri-apps", "cli", "tauri.js"), ...rest];
if (features.length) cmd.splice(2, 0, "--features", features.join(","));

const fetch = spawnSync(process.execPath, [path.join(root, "scripts", "fetch-models.mjs")], { cwd: root, stdio: "inherit", env });
if (fetch.status) process.exit(fetch.status);
process.exit(spawnSync(process.execPath, cmd, { cwd: root, stdio: "inherit", env }).status ?? 1);
