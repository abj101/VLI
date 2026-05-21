# JARVIS (Tauri + React + TypeScript)

## Prerequisites (Windows)

- [Rust](https://rustup.rs/) stable, [Node.js](https://nodejs.org/) LTS
- **Whisper / `whisper-rs`:** **CMake** (`winget install Kitware.CMake`), **LLVM** (`winget install LLVM.LLVM` — `libclang.dll` in `C:\Program Files\LLVM\bin`), and **MSVC** (**VS 2022** Build Tools or full VS with **Windows SDK**). `whisper-rs-sys` runs **bindgen** at build time; it needs `LIBCLANG_PATH` plus MSVC/UCRT include paths (`BINDGEN_EXTRA_CLANG_ARGS`). If bindgen cannot find `stdbool.h`, it falls back to bundled **Linux** `bindings.rs` and you get **`error[E0080]: attempt to compute 12_usize - 16_usize`**. **`npm run tauri dev` / `build`** set env via `scripts/whisper-gpu/run-tauri.mjs` and **exit early** if bindgen prerequisites are missing (instead of compiling for 15+ minutes then failing). For bare **`cargo check`** / rust-analyzer: run from **`jarvis/`**, then **`npm install`** or **`npm run sync:cargo-win-env`** (writes **`src-tauri/.cargo/config.local.toml`** and **`rust-analyzer.toml`**). After any log line **`Using bundled bindings.rs`**: **`npm run cargo -- clean -p whisper-rs-sys --manifest-path src-tauri/Cargo.toml`** before the next build. CUDA builds use **NMake + nvcc** when `whisper-cuda` is auto-selected.
- **Piper TTS (`Speak` action):** install/download `piper.exe` (Windows) or `piper` (macOS/Linux) and one `.onnx` voice model. Configure with env vars `JARVIS_PIPER_BIN` and `JARVIS_PIPER_MODEL` (or `PIPER_BIN` / `PIPER_MODEL`). Fallback search paths include `src-tauri/resources/piper/`.
- **PATH / discovery:** Rust builds read `src-tauri/.cargo/config.toml`, which defaults `CMAKE` to `cmake` and expects your shell `PATH` to resolve the executable. For shells outside the editor, add your CMake install location to `PATH` or export `CMAKE`.
- **Microphone** permission for the dev or packaged app
- **PowerShell:** If you run `cargo` yourself in PowerShell, do not append `2>&1` to the command — Cargo uses stderr for progress lines, and merging streams makes PowerShell show spurious `RemoteException` / `CategoryInfo` output even when the build succeeded. Use the command’s exit code (`$LASTEXITCODE`) to detect failure.

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
- `npm run tauri:dev:cpu` / `npm run tauri:build:cpu` — same as above but **`WHISPER_GPU_BACKEND=none`** (fast CPU Whisper; use for UI work)
- `npm run tauri build` — release bundle with auto-selected Whisper GPU backend (run `.\scripts\download-model.ps1` first so the Whisper weights are present)
- `npm run tauri:dev` / `npm run tauri:build` — explicit aliases to the same wrapper behavior
- `WHISPER_GPU_BACKEND=auto|metal|cuda|vulkan|none` — optional override for deterministic CI/repro builds (`auto` default)
- **`npm run sync:cargo-win-env`** writes bindgen env only (no `CMAKE_GENERATOR`) so rust-analyzer / `cargo check` does not invalidate a **CUDA (NMake)** CMake cache from `tauri dev`. For CUDA compile checks use **`npm run test:cargo-whisper-cuda`**.

Rust (from `jarvis/src-tauri/`):

- `cargo fmt --check`
- `cargo clippy -- -D warnings`
- `cargo test`

## CI / release checklist (local or automation)

Run in order after a clean checkout (with Rust + Node + CMake + MSVC or Xcode as above):

1. `cd jarvis` → `npm ci` (or `npm install`)
2. `npm run lint`
3. `npm test`
4. `npm run build`
5. `cd src-tauri` → `cargo fmt --check`
6. `cargo clippy -- -D warnings`
7. `cargo test`
8. `cd ..` → `.\scripts\download-model.ps1`
9. `npm run tauri build`

### Whisper compile: “frozen” terminal?

`whisper-rs-sys` with **`whisper-cuda`** compiles many CUDA kernels on the **first** build — often **20–45+ minutes** on Windows; Cargo progress may sit near the end during link. That is normal, not a hung app.

- Run **one** `npm run tauri dev` or `cargo` at a time. Parallel builds block on `target/` (“Blocking waiting for file lock”) and look frozen.
- Leftover `target/**/.cargo-lock` after Ctrl+C is removed automatically; if a real build is still running, wait or stop it. Override: `WHISPER_IGNORE_CARGO_LOCK=1` (risky).
- The launcher prints a **heartbeat** every minute during long builds and warns on first CUDA compile.
- If bindgen failed earlier, clean before retry: `npm run cargo -- clean -p whisper-rs-sys --manifest-path src-tauri/Cargo.toml`

### Whisper GPU backend auto-selection

`scripts/whisper-gpu/run-tauri.mjs` chooses one backend per artifact:

- macOS host -> `whisper-metal`
- Windows/Linux + NVIDIA GPU + CUDA toolkit -> `whisper-cuda`
- Other GPUs (or NVIDIA without CUDA) + Vulkan SDK -> `whisper-vulkan`
- Missing toolchains -> CPU-only Whisper (`whisper` GPU features off; warning logged)

Install **CUDA** or **Vulkan SDK** manually (see NVIDIA / Khronos docs); set `CUDA_PATH` or `VULKAN_SDK` if not auto-discovered. `WHISPER_GPU_BACKEND=none` or **`npm run tauri:dev:cpu`** forces a faster CPU-only dev build.

**Rebuild loop:** alternating bare `cargo check` (after sync) with `npm run tauri dev` on NVIDIA (CUDA + NMake) forces a full `whisper-rs-sys` rebuild. Stay on one workflow until `ggml-cuda.lib` exists, then incremental dev is typically ~1–2 minutes.

The wrapper runs the Tauri CLI via `node node_modules/@tauri-apps/cli/tauri.js` (not `tauri.cmd`) after GPU detection and bindgen preflight on Windows.

Legacy entry: `scripts/tauri-whisper-gpu.mjs` re-exports the same launcher.

```powershell
npm run tauri dev
npm run tauri build
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

- **HUD (overlay):** global shortcut (default **Ctrl+Shift+J**, configurable in Settings → Hotkeys) shows or toggles the HUD; **Dismiss voice overlay** (default **escape**, also under Hotkeys) stops the session and hides the HUD. The two shortcuts must use different combos.
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

