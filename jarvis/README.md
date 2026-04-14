# JARVIS (Tauri + React + TypeScript)

## Prerequisites (Windows)

- [Rust](https://rustup.rs/) stable, [Node.js](https://nodejs.org/) LTS
- **Whisper / `whisper-rs`:** **CMake** (e.g. `winget install Kitware.CMake`) and **LLVM 18.x** (winget; for bindgen set user env `LIBCLANG_PATH` to `C:\Program Files\LLVM\bin` so `libclang.dll` is found), plus a **MSVC** C++ build environment (Visual Studio “Desktop development with C++” workload, or Build Tools). `whisper-rs` builds native `whisper.cpp` sources via CMake.
- **Piper TTS (`Speak` action):** install/download `piper.exe` and one `.onnx` voice model. Configure with env vars `JARVIS_PIPER_BIN` and `JARVIS_PIPER_MODEL` (or `PIPER_BIN` / `PIPER_MODEL`). Fallback search paths include `src-tauri/resources/piper/`.
- **PATH / discovery:** This repo prepends **`%ProgramFiles%\CMake\bin`** and **`%ProgramFiles%\LLVM\bin`** to `PATH` in **`.vscode/settings.json`** (repo root and under `jarvis/`) so Cursor/VS Code integrated terminals find `cmake` and LLVM. Rust builds also read **`src-tauri/.cargo/config.toml`**, which sets **`CMAKE`** to the default Kitware path when that file exists (override with your own `CMAKE` env if CMake is installed elsewhere). For shells outside the editor, add those directories to your user **PATH** or export **`CMAKE`**.
- **Microphone** permission for the dev or packaged app

## Whisper model (bundled path)

Weights are **not** committed (see root `.gitignore` `*.bin`). From the `jarvis` folder:

```powershell
.\scripts\download-model.ps1
```

This writes `src-tauri/resources/ggml-tiny.en.bin`, which `tauri.conf.json` lists under `bundle.resources`. Run the script before `npm run tauri build` so bundling can include the file.

## Piper voice model (for `Speak`)

`Speak` looks for Piper runtime/model in this order:

- Env vars: `JARVIS_PIPER_BIN` + `JARVIS_PIPER_MODEL` (or `PIPER_BIN` + `PIPER_MODEL`)
- Bundled/resources fallback: `src-tauri/resources/piper/piper.exe` and `src-tauri/resources/piper/en_US-amy-medium.onnx`

Successful synth output is cached in app data under `tts-cache/` to avoid repeated synthesis for same text + voice.

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

## Phase 2 manual verification (Windows)

Use this gate before calling a Phase 2 build releasable:

1. Trigger fuzzy phrase (typo) and confirm intended command still matches/executes.
2. Run a multi-action chain (`OpenApp` -> `Wait` -> `OpenUrl`) and confirm strict order.
3. Run at least one command with `Speak`; confirm audible output or controlled Piper-missing error.
4. Run a `SubPrompt` command and provide follow-up input; confirm templated follow-up action executes.
5. Re-run `SubPrompt` and let it timeout (or cancel); confirm safe terminal HUD phase, no crash/deadlock.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

