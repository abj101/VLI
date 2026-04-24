# JARVIS (Tauri + React + TypeScript)

## Prerequisites (Windows)

- [Rust](https://rustup.rs/) stable, [Node.js](https://nodejs.org/) LTS
- **Whisper / `whisper-rs`:** **CMake** (e.g. `winget install Kitware.CMake`) and **LLVM 18.x** (winget; for bindgen set user env `LIBCLANG_PATH` to `C:\Program Files\LLVM\bin` so `libclang.dll` is found), plus a **MSVC** C++ build environment (Visual Studio “Desktop development with C++” workload, or Build Tools). `whisper-rs` builds native `whisper.cpp` sources via CMake.
- **Piper TTS (`Speak` action):** install/download `piper.exe` (Windows) or `piper` (macOS/Linux) and one `.onnx` voice model. Configure with env vars `JARVIS_PIPER_BIN` and `JARVIS_PIPER_MODEL` (or `PIPER_BIN` / `PIPER_MODEL`). Fallback search paths include `src-tauri/resources/piper/`.
- **PATH / discovery:** Rust builds read `src-tauri/.cargo/config.toml`, which defaults `CMAKE` to `cmake` and expects your shell `PATH` to resolve the executable. For shells outside the editor, add your CMake install location to `PATH` or export `CMAKE`.
- **Microphone** permission for the dev or packaged app

## Prerequisites (macOS)

- [Rust](https://rustup.rs/) stable, [Node.js](https://nodejs.org/) LTS
- Xcode CLI tools: `xcode-select --install`
- Homebrew packages: `brew install cmake llvm`
- Add to shell profile (for `bindgen`): `export LIBCLANG_PATH="$(brew --prefix llvm)/lib"`
- Ensure Homebrew binaries are on PATH (Apple Silicon default): `export PATH="/opt/homebrew/bin:$PATH"`
- **Piper TTS (`Speak` action):** install/download `piper` (no `.exe`) and one `.onnx` voice model; env vars and model path behavior are the same as Windows
- **Microphone** permission for Terminal/IDE and the app

## Whisper model (bundled path)

Weights are **not** committed (see root `.gitignore` `*.bin`). From the `jarvis` folder:

```powershell
.\scripts\download-model.ps1
```

macOS/Linux equivalent:

```bash
./scripts/download-model.sh
```

This writes `src-tauri/resources/ggml-tiny.en.bin`, which `tauri.conf.json` lists under `bundle.resources`. Run the script before `npm run tauri build` so bundling can include the file.

## Piper voice model (for `Speak`)

`Speak` looks for Piper runtime/model in this order:

- Env vars: `JARVIS_PIPER_BIN` + `JARVIS_PIPER_MODEL` (or `PIPER_BIN` + `PIPER_MODEL`)
- Bundled/resources fallback:
  - Windows: `src-tauri/resources/piper/piper.exe`
  - macOS/Linux: `src-tauri/resources/piper/piper`
  - Model (all platforms): `src-tauri/resources/piper/en_US-amy-medium.onnx`

Successful synth output is cached in app data under `tts-cache/` to avoid repeated synthesis for same text + voice.

## Commands (from `jarvis/`)

Convention: clone the repo, then `**cd jarvis**` for every Node/npm/Tauri command below. Raw `cargo` commands use `**cd jarvis/src-tauri**`.

- `npm install`
- `npm run lint` — ESLint (TypeScript + React)
- `npm test` — Vitest
- `npm run build` — `tsc` + Vite production bundle
- `npm run dev` — Vite only
- `npm run tauri dev` — full app with auto-selected Whisper GPU backend (`metal`/`cuda`/`vulkan`/CPU fallback) and detected GPU vendor logging
- `npm run tauri build` — release bundle with auto-selected Whisper GPU backend (run `.\scripts\download-model.ps1` first so the Whisper weights are present)
- `npm run tauri:dev` / `npm run tauri:build` — explicit aliases to the same wrapper behavior
- `WHISPER_GPU_BACKEND=auto|metal|cuda|vulkan|none` — optional override for deterministic CI/repro builds (`auto` default)

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

### Whisper GPU backend auto-selection

`scripts/tauri-whisper-gpu.mjs` chooses one backend per artifact:

- macOS host -> `whisper-metal`
- Windows/Linux + NVIDIA GPU + CUDA toolchain (`CUDA_PATH`, `nvcc`, or auto-discovered default Windows CUDA install under `C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA`) -> `whisper-cuda`
- Other GPUs (or NVIDIA without CUDA) + Vulkan toolchain -> `whisper-vulkan` (`VULKAN_SDK` is required on Windows; auto-discovered from common install paths when possible)
- If no backend prerequisites are present -> CPU-only Whisper build (explicit warning logged)

The wrapper runs the Tauri CLI via `node node_modules/@tauri-apps/cli/tauri.js` (not `tauri.cmd`) so `npm run tauri build|dev` reliably continues into the actual build on Windows after GPU detection.

On **Windows**, when an **NVIDIA** GPU is detected and the **CUDA Toolkit** is missing, the wrapper first asks whether to install `Nvidia.CUDA` via winget (before Vulkan/other prompts).

When `tauri dev` / `tauri build` needs **Vulkan** or **CUDA** for Whisper and the SDK/toolkit is missing, the wrapper can also prompt:

- `Install … with winget now? [y/N]` — answering `y` runs `winget install` for `KhronosGroup.VulkanSDK` or `Nvidia.CUDA` (large download; may require admin / UAC).

Non-interactive terminals (CI) skip prompts; set `WHISPER_SKIP_PREREQ_PROMPT=1` to skip explicitly, or install SDKs / set `VULKAN_SDK` / `CUDA_PATH` yourself.

Windows wrappers:

```powershell
.\scripts\build-tauri-with-whisper-gpu.ps1 build
.\scripts\build-tauri-with-whisper-gpu.ps1 dev
```

Manual QA matrix (GPU path):

- Windows + NVIDIA (CUDA installed): backend selects `whisper-cuda`, Settings shows GPU available.
- Windows + Intel/AMD GPU: backend selects `whisper-vulkan`, Settings shows GPU available when Vulkan loader exists.
- macOS Apple Silicon: backend selects `whisper-metal`, Settings shows GPU available.
- Any host without required toolchains: CPU-only build, Settings explains GPU backend is unavailable in current build/runtime.

Packaged artifacts appear under `src-tauri/target/release/bundle/` (e.g. `.exe` installer / MSI, depending on Tauri bundler settings).

## Phase 3 — Command editor (Windows)

The **React command editor** is a second window (not the HUD). Open it from the **system tray**: **Open Editor** (above Pause/Resume). If the editor is already open, choosing **Open Editor** again **focuses** the existing window instead of opening a duplicate.

### What you can do

- **Left panel:** list of command nodes from SQLite — select a row to edit, toggle enabled, delete (with confirmation), reorder rows (drag handle or ↑/↓), or use **+** for a new command.
- **Right panel:** edit name, trigger phrases (tags), fuzzy threshold, action chain (all action types + optional sub-prompt chain), then **Save** / **Cancel**.
- **Header (gear):** **Settings** — global hotkey (persisted; re-registered live), default fuzzy threshold for nodes without an override, theme (`dark` / `light` / `system`) stored in the `settings` table.

### Keyboard and shortcuts

- **HUD (overlay):** global hotkey (default **Ctrl+Shift+J**, configurable in Settings) starts listening; **Esc** stops (same as the on-screen stop control). Shortcut hint under the stop button is **9px** mono per design spec.
- **Editor window:** standard **Tab** / **Shift+Tab** focus order; form fields and buttons have no separate global chord beyond OS defaults. After changing the hotkey in Settings, the new combo applies immediately after a successful save.

### Migrations

SQLite schema changes are **additive** migrations run at startup (e.g. `sort_order` on `command_nodes`). See `**src-tauri/MIGRATIONS.md`** for the log and idempotency notes. Copy existing user DBs forward without destructive resets for Phase 3 changes.

### Tests and coverage

From `jarvis/`: `npm run test` runs Vitest. Coverage thresholds (**≥70%** lines on `editorStore`, `NodeForm.logic`, `ActionChain.logic`) are enforced when you run `npm run test:coverage`.

## Phase 2 manual verification (Windows)

Use this gate before calling a Phase 2 build releasable:

1. Trigger fuzzy phrase (typo) and confirm intended command still matches/executes.
2. Run a multi-action chain (`OpenApp` -> `Wait` -> `OpenUrl`) and confirm strict order.
3. Run at least one command with `Speak`; confirm audible output or controlled Piper-missing error.
4. Run the `subprompt test` voice command, answer with a topic (for example `rust`), and confirm browser opens GitHub search results for that topic.
5. Re-run `SubPrompt` and let it timeout (or cancel); confirm safe terminal HUD phase, no crash/deadlock.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

