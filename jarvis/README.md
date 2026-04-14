# JARVIS (Tauri + React + TypeScript)

## Prerequisites (Windows)

- [Rust](https://rustup.rs/) stable, [Node.js](https://nodejs.org/) LTS
- **Whisper / `whisper-rs`:** CMake plus a **MSVC** C++ build environment (Visual Studio “Desktop development with C++” workload, or Build Tools). `whisper-rs` builds native `whisper.cpp` sources via CMake.
- **Microphone** permission for the dev or packaged app

## Whisper model (bundled path)

Weights are **not** committed (see root `.gitignore` `*.bin`). From the `jarvis` folder:

```powershell
.\scripts\download-model.ps1
```

This writes `src-tauri/resources/ggml-tiny.en.bin`, which `tauri.conf.json` lists under `bundle.resources`. Run the script before `npm run tauri build` so bundling can include the file.

## Commands (from `jarvis/`)

- `npm install`
- `npm run dev` — Vite only
- `npm run tauri dev` — full app
- `cd src-tauri; cargo test audio::; cargo clippy -- -D warnings`

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
