# JARVIS (Tauri + React + TypeScript)

## Prerequisites (Windows)

- [Rust](https://rustup.rs/) stable, [Node.js](https://nodejs.org/) LTS
- **Whisper / `whisper-rs`:** **CMake** (e.g. `winget install Kitware.CMake`) and **LLVM 18.x** (winget; for bindgen set user env `LIBCLANG_PATH` to `C:\Program Files\LLVM\bin` so `libclang.dll` is found), plus a **MSVC** C++ build environment (Visual Studio “Desktop development with C++” workload, or Build Tools). `whisper-rs` builds native `whisper.cpp` sources via CMake.
- **PATH / discovery:** This repo prepends **`%ProgramFiles%\CMake\bin`** and **`%ProgramFiles%\LLVM\bin`** to `PATH` in **`.vscode/settings.json`** (repo root and under `jarvis/`) so Cursor/VS Code integrated terminals find `cmake` and LLVM. Rust builds also read **`src-tauri/.cargo/config.toml`**, which sets **`CMAKE`** to the default Kitware path when that file exists (override with your own `CMAKE` env if CMake is installed elsewhere). For shells outside the editor, add those directories to your user **PATH** or export **`CMAKE`**.
- **Microphone** permission for the dev or packaged app

## Whisper model (bundled path)

Weights are **not** committed (see root `.gitignore` `*.bin`). From the `jarvis` folder:

```powershell
.\scripts\download-model.ps1
```

This writes `src-tauri/resources/ggml-tiny.en.bin`, which `tauri.conf.json` lists under `bundle.resources`. Run the script before `npm run tauri build` so bundling can include the file.

## Commands (from `jarvis/`)

Convention: clone the repo, then **`cd jarvis`** for every Node/npm/Tauri command below. Raw `cargo` commands use **`cd jarvis/src-tauri`**.

- `npm install`
- `npm run lint` — ESLint (TypeScript + React)
- `npm test` — Vitest
- `npm run build` — `tsc` + Vite production bundle
- `npm run dev` — Vite only
- `npm run tauri dev` — full app
- `npm run tauri build` — release bundle (run `.\scripts\download-model.ps1` first so the Whisper weights are present)

Rust (from `jarvis/src-tauri/`):

- `cargo fmt --check`
- `cargo clippy -- -D warnings`
- `cargo test`

## CI / release checklist (local or automation)

Run in order after a clean checkout (with Rust + Node + CMake + MSVC + LLVM as above):

1. `cd jarvis` → `npm ci` (or `npm install`)
2. `npm run lint`
3. `npm test`
4. `npm run build`
5. `cd src-tauri` → `cargo fmt --check`
6. `cargo clippy -- -D warnings`
7. `cargo test`
8. `cd ..` → `.\scripts\download-model.ps1`
9. `npm run tauri build`

Packaged artifacts appear under `src-tauri/target/release/bundle/` (e.g. `.exe` installer / MSI, depending on Tauri bundler settings).

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

