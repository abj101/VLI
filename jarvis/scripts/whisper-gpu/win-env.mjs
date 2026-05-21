/**
 * Windows MSVC / bindgen / CMake env for whisper-rs-sys (Tauri launcher + Cargo config sync).
 */

import { spawnSync } from "child_process";
import fs from "fs";
import path from "path";

export function parseVersionParts(name) {
  return name.split(".").map((p) => Number.parseInt(p, 10) || 0);
}

export function compareVersionNamesDesc(a, b) {
  const av = parseVersionParts(a);
  const bv = parseVersionParts(b);
  const n = Math.max(av.length, bv.length);
  for (let i = 0; i < n; i += 1) {
    const x = av[i] ?? 0;
    const y = bv[i] ?? 0;
    if (x !== y) return y - x;
  }
  return 0;
}

export function discoverWindowsMsvcHostX64BinDir() {
  const roots = [
    "C:\\Program Files (x86)\\Microsoft Visual Studio\\2022\\BuildTools\\VC\\Tools\\MSVC",
    "C:\\Program Files\\Microsoft Visual Studio\\2022\\Community\\VC\\Tools\\MSVC",
    "C:\\Program Files\\Microsoft Visual Studio\\2022\\Professional\\VC\\Tools\\MSVC",
    "C:\\Program Files\\Microsoft Visual Studio\\2022\\Enterprise\\VC\\Tools\\MSVC",
  ];
  for (const root of roots) {
    if (!fs.existsSync(root)) continue;
    const versions = fs
      .readdirSync(root, { withFileTypes: true })
      .filter((d) => d.isDirectory())
      .map((d) => d.name)
      .sort(compareVersionNamesDesc);
    for (const version of versions) {
      const hostBin = path.join(root, version, "bin", "Hostx64", "x64");
      if (fs.existsSync(path.join(hostBin, "cl.exe"))) {
        return hostBin;
      }
    }
  }
  return null;
}

export function discoverWindowsMsvcIncludeDir() {
  const hostBin = discoverWindowsMsvcHostX64BinDir();
  if (!hostBin) return null;
  const inc = path.normalize(path.join(hostBin, "..", "..", "..", "include"));
  return fs.existsSync(path.join(inc, "vcruntime.h")) ? inc : null;
}

/** @returns {string[]} */
export function discoverWindowsKitsBindgenIncludes() {
  const incRoot = path.join(
    process.env["ProgramFiles(x86)"] ?? "C:\\Program Files (x86)",
    "Windows Kits",
    "10",
    "Include",
  );
  if (!fs.existsSync(incRoot)) return [];
  const versions = fs
    .readdirSync(incRoot, { withFileTypes: true })
    .filter((d) => d.isDirectory())
    .map((d) => d.name)
    .filter((n) => /^\d+\.\d+\.\d+/.test(n))
    .sort(compareVersionNamesDesc);
  for (const ver of versions) {
    const base = path.join(incRoot, ver);
    const ucrt = path.join(base, "ucrt");
    if (!fs.existsSync(path.join(ucrt, "corecrt.h"))) continue;
    const out = [ucrt, path.join(base, "shared"), path.join(base, "um")].filter((p) =>
      fs.existsSync(p),
    );
    return out.length >= 1 ? out : [];
  }
  return [];
}

export function clangPath(p) {
  return path.resolve(p).replace(/\\/g, "/");
}

export function windowsShortPath(longPath) {
  const resolved = path.resolve(longPath);
  if (process.platform !== "win32" || !resolved.includes(" ")) {
    return clangPath(resolved);
  }
  const ps = spawnSync(
    "powershell.exe",
    [
      "-NoProfile",
      "-Command",
      `(New-Object -ComObject Scripting.FileSystemObject).GetFolder('${resolved.replace(/'/g, "''")}').ShortPath`,
    ],
    { encoding: "utf8", windowsHide: true },
  );
  const line = ps.stdout?.trim();
  if (line && fs.existsSync(line)) {
    return clangPath(line);
  }
  return clangPath(resolved);
}

export function windowsBindgenExtraClangArgs() {
  const msvcInc = discoverWindowsMsvcIncludeDir();
  const kits = discoverWindowsKitsBindgenIncludes();
  if (!msvcInc) return null;
  const includeRoots = [msvcInc, ...kits];
  const hasStdbool = includeRoots.some((dir) => fs.existsSync(path.join(dir, "stdbool.h")));
  if (!hasStdbool) return null;
  const parts = includeRoots.map((p) => {
    const n = windowsShortPath(p);
    return n.includes(" ") ? `-isystem "${n}"` : `-isystem ${n}`;
  });
  parts.push("-std=c11");
  return parts.join(" ");
}

export function validateWindowsWhisperBindgenEnv() {
  const errors = [];
  const warnings = [];
  if (process.platform !== "win32") {
    return { ok: true, errors, warnings };
  }
  if (!windowsDefaultLibClangDir(process.env.LIBCLANG_PATH)) {
    errors.push(
      "LLVM libclang not found (winget install LLVM.LLVM; expect libclang.dll under Program Files/LLVM/bin)",
    );
  }
  if (!discoverWindowsMsvcIncludeDir()) {
    errors.push("MSVC include dir not found (install VS 2022 Build Tools with C++ workload)");
  }
  if (discoverWindowsKitsBindgenIncludes().length === 0) {
    errors.push("Windows 10/11 SDK include dirs not found (install Windows SDK via VS installer)");
  }
  if (!windowsBindgenExtraClangArgs()) {
    errors.push(
      "stdbool.h not found under MSVC/SDK includes (bindgen will fall back to Linux bindings → E0080)",
    );
  }
  if (errors.length > 0) {
    warnings.push(
      "After fixing: npm run sync:cargo-win-env && npm run cargo -- clean -p whisper-rs-sys --manifest-path src-tauri/Cargo.toml",
    );
    warnings.push(
      "If the build log shows 'Using bundled bindings.rs', clean whisper-rs-sys before the next build.",
    );
  }
  return { ok: errors.length === 0, errors, warnings };
}

/**
 * Exit before Cargo when bindgen prerequisites are missing (avoids 15+ min CUDA compile then E0080).
 * @param {string} [label]
 */
export function assertWindowsWhisperBindgenEnv(label = "whisper-gpu") {
  if (process.platform !== "win32") return;
  const check = validateWindowsWhisperBindgenEnv();
  if (check.ok) return;
  console.error(`${label}: cannot build whisper-rs-sys — bindgen prerequisites missing:`);
  for (const e of check.errors) console.error(`  - ${e}`);
  for (const w of check.warnings) console.warn(`  ${w}`);
  process.exit(1);
}

export function windowsDefaultLibClangDir(alreadySet) {
  const trimmed = alreadySet?.trim();
  if (trimmed) {
    const dll = path.join(trimmed, "libclang.dll");
    if (fs.existsSync(dll)) return trimmed;
  }
  const candidates = [
    process.env.LLVM_PATH ? path.join(process.env.LLVM_PATH, "bin") : "",
    path.join(process.env.ProgramFiles ?? "C:\\Program Files", "LLVM", "bin"),
    path.join(process.env["ProgramFiles(x86)"] ?? "C:\\Program Files (x86)", "LLVM", "bin"),
  ].filter((p) => typeof p === "string" && p.length > 0);
  for (const dir of candidates) {
    if (fs.existsSync(path.join(dir, "libclang.dll"))) return dir;
  }
  return null;
}

export function buildWindowsWhisperCargoEnv(base = process.env, opts = {}) {
  const force = opts.force === true;
  const includeCmakeGenerator = opts.includeCmakeGenerator !== false;
  /** @type {Record<string, string>} */
  const out = {};
  if (includeCmakeGenerator && !base.CMAKE_GENERATOR?.trim()) {
    out.CMAKE_GENERATOR = "Visual Studio 17 2022";
  }
  const libClang = windowsDefaultLibClangDir(base.LIBCLANG_PATH);
  if (libClang && (force || !base.LIBCLANG_PATH?.trim())) {
    out.LIBCLANG_PATH = libClang;
  }
  const bindgenExtra = windowsBindgenExtraClangArgs();
  if (bindgenExtra) {
    const prev = base.BINDGEN_EXTRA_CLANG_ARGS?.trim();
    const merged = force || !prev ? bindgenExtra : `${prev} ${bindgenExtra}`;
    if (force || !base.BINDGEN_EXTRA_CLANG_ARGS?.trim()) {
      out.BINDGEN_EXTRA_CLANG_ARGS = merged;
    }
    if (force || !base.BINDGEN_EXTRA_CLANG_ARGS_x86_64_pc_windows_msvc?.trim()) {
      out.BINDGEN_EXTRA_CLANG_ARGS_x86_64_pc_windows_msvc = merged;
    }
    if (force || !base["BINDGEN_EXTRA_CLANG_ARGS_x86_64-pc-windows-msvc"]?.trim()) {
      out["BINDGEN_EXTRA_CLANG_ARGS_x86_64-pc-windows-msvc"] = merged;
    }
  }
  return out;
}

export function tomlBasicString(value) {
  return JSON.stringify(value);
}

export function tomlEnvValue(value) {
  return tomlBasicString(value);
}
