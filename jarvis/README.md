# JARVIS (Tauri + React + TypeScript)

## Prerequisites (Windows)

- [Rust](https://rustup.rs/) stable, [Node.js](https://nodejs.org/) LTS
- **Whisper / `whisper-rs`:** **CMake** (`winget install Kitware.CMake`), **LLVM** (`winget install LLVM.LLVM` — `libclang.dll` in `C:\Program Files\LLVM\bin`), and **MSVC** (**VS 2022** Build Tools or full VS with **Windows SDK**). Optional but recommended for CUDA compile speed: **Ninja** (`winget install Ninja-build.Ninja` — must be on `PATH` as `ninja.exe`). `whisper-rs-sys` runs **bindgen** at build time; it needs `LIBCLANG_PATH` plus MSVC/UCRT include paths (`BINDGEN_EXTRA_CLANG_ARGS`). If bindgen cannot find `stdbool.h`, it falls back to bundled **Linux** `bindings.rs` and you get **`error[E0080]: attempt to compute 12_usize - 16_usize`**. **`npm run tauri dev` / `build`** set env via `scripts/whisper-gpu/run-tauri.mjs` and **exit early** if bindgen prerequisites are missing (instead of compiling for 15+ minutes then failing). For bare **`cargo check`** / rust-analyzer: run from **`jarvis/`**, then **`npm install`** or **`npm run sync:cargo-win-env`** (writes **`src-tauri/.cargo/config.local.toml`** and **`rust-analyzer.toml`**). After any log line **`Using bundled bindings.rs`**: **`npm run cargo -- clean -p whisper-rs-sys --manifest-path src-tauri/Cargo.toml`** before the next build. CUDA builds use **Ninja + nvcc + cl.exe** when `ninja.exe` is on `PATH`, else **NMake + nvcc + cl.exe**.
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
- **Microphone** permission for the jarvis app (dev and packaged builds use bundle id **`com.jarvis.app`** in System Settings)
- On first launch macOS should prompt for microphone access (`Info.plist` includes `NSMicrophoneUsageDescription`). If voice/dictation/wake word are silent and you never saw a prompt:
  1. **Reset stale TCC** (required once after upgrading mic permission code): `tccutil reset Microphone com.jarvis.app`
  2. Rebuild/restart: `npm run tauri dev` (dev binaries are ad-hoc signed with `com.jarvis.app` + mic entitlements on each link)
  3. Allow the prompt, or open **System Settings → Privacy & Security → Microphone** and enable **jarvis**
- If you run the binary with raw `cargo run` outside `npm run tauri dev`, you may also need to enable **Terminal** or **Cursor** in Microphone settings.
- Global hotkeys (`ctrl+j` voice HUD if configured in Settings, `ctrl+shift+d` dictation) need the app running and not paused. **`ctrl+j` is often taken by Cursor/VS Code** (toggle panel) — change the voice HUD hotkey in Settings if it never fires while the editor is focused.

### macOS microphone (production builds)

After `npm run tauri build`, verify the bundled app includes mic permission keys:

```bash
plutil -p src-tauri/target/release/bundle/macos/jarvis.app/Contents/Info.plist | grep NSMicrophone
codesign -d --entitlements - src-tauri/target/release/bundle/macos/jarvis.app 2>&1 | grep audio-input
```

Signed/notarized releases need a Developer ID certificate. Set `APPLE_SIGNING_IDENTITY` (or `bundle.macOS.signingIdentity` in `tauri.conf.json`) before building. See [Tauri macOS code signing](https://v2.tauri.app/distribute/sign/macos/). `Entitlements.plist` includes `com.apple.security.device.audio-input` and `hardenedRuntime` is enabled in `tauri.conf.json`.

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

## On-device LLM models (router + composer)

`npm run tauri:dev` / `npm run tauri:build` fetch models **before** Tauri starts (so Cargo can bundle GGUF paths and Vite is not blocked). First run downloads ~2.4 GB total; later runs skip existing files.

| Role | Model file | Size | When it runs |
| --- | --- | --- | --- |
| **Voice router** (Tier 2 tool pick) | `qwen2.5-0.5b-instruct-q4_k_m.gguf` | ~400 MB | Hot path after wake / fuzzy match |
| **Command composer** (Commands tab NL → draft) | `qwen2.5-3b-instruct-q4_k_m.gguf` | ~2 GB | Cold path — editor **+** draft row **Describe** → **Generate** |

From the `jarvis` folder (first fetch only; skips files already on disk):

```powershell
npm run fetch-models
```

Or fetch LLM weights only: `npm run fetch-llm-models`. PowerShell wrappers `scripts/download-router-model.ps1` and `scripts/download-composer-model.ps1` call the same fetcher.

The composer model is **not** interchangeable with the router: the 3B model emits full action chains and template variables; the 0.5B model only routes to a small builtin tool catalog. Opening the **editor** warms the composer model in the background (not at app launch). First **Generate** reuses the resident worker; GPU decode is strongly recommended — CPU works but can take tens of seconds per draft. Optional settings override: `llm_composer_model_path`, `llm_router_model_path` (stored in SQLite `settings`).

### Command composer examples (Commands tab → **+** → **Describe** → **Generate**)

Golden-path prompts to try after models are fetched:

| You describe | Typical result |
| --- | --- |
| `When I say "open notepad", launch Notepad snapped to the left` | **Command** — phrase trigger + `open_target` with placement |
| `When I say open notepad then open notepad` | **Command** — single `open_target` for Notepad (trigger echoed in speech must not duplicate the action) |
| `When I say notepad, open notepad fullscreen, new note, paste Hello World, 30 second timer` | **Command** — `open_target` (maximize) → `^n` new note → clipboard + `^v` paste → `wait` 30s |
| `Speak whatever text I give you` | **Tool** — parameterized `speak` with a `text` parameter (no voice trigger) |
| `When I say fetch, http get the URL I say after the trigger` | **Command** (prefix) — `http_get` on `{{remainder}}` |

Review the draft formula (or tool preview), edit if needed, then **Save** / **Create tool**. If validation fails after the automatic repair pass, use **Regenerate with fix** to try again with the same description.

### Composer eval suite

Fixture-driven regression tests live in `src-tauri/src/llm/composer_eval_cases.json`. Mock-infer cases run in CI without the GGUF model; live eval hits the real composer when wired.

| Command | What it runs |
| --- | --- |
| `npm run test:composer` | `composer_eval` + `composer_expect` unit/fixture tests (skipped `#[ignore]` live test) |
| `npm run test:composer:live` | `composer_live_eval` with `--ignored --nocapture` (needs `llm-local` + composer GGUF on disk) |
| `npm run eval:composer` | Same live run as above, plus pass/fail summary table |
| `npm run eval:composer -- --report` | Live eval + writes `composer-eval-report.json` (timestamp, pass count) |

Fetch models first (`npm run fetch-llm-models`) before live eval. Mock suite is fast and safe for every PR.

## Commands (from `jarvis/`)

Convention: clone the repo, then `**cd jarvis**` for every Node/npm/Tauri command below. Raw `cargo` commands use `**cd jarvis/src-tauri**`.

- `npm install`
- `npm run lint` — ESLint (TypeScript + React)
- `npm test` — Vitest
- `npm run test:composer` — composer fixture + expectation tests (mock infer, no GGUF)
- `npm run test:composer:live` — ignored live composer eval against real model
- `npm run build` — `tsc` + Vite production bundle
- `npm run dev` — Vite only
- `npm run tauri dev` — full app; **`dev` and `build` auto-select GPU** (`metal`/`cuda`/`vulkan`, CPU fallback if toolchains missing).
- `npm run tauri:dev:gpu` / `npm run tauri:build:gpu` — same as default (`WHISPER_GPU_BACKEND=auto`; first CUDA build often 20–45+ min on Windows).
- `npm run tauri:dev:cpu` / `npm run tauri:build:cpu` — faster CPU-only dev: **`WHISPER_GPU_BACKEND=none`**
- `npm run tauri build` — release bundle with auto-selected Whisper GPU backend (run `.\scripts\download-model.ps1` first so the Whisper weights are present)
- `npm run tauri:dev` / `npm run tauri:build` — explicit aliases to the same wrapper behavior
- `WHISPER_GPU_BACKEND=auto|metal|cuda|vulkan|none` — optional override for deterministic CI/repro builds (`auto` default)
- **`npm run sync:cargo-win-env`** (runs on **`npm install`** on **Windows only**) fills **`src-tauri/.cargo/config.local.toml`** with bindgen + **`CMAKE_GENERATOR`** for bare **`cargo check`** only. **`npm run tauri *`** sets generator via process env (not tracked **`config.toml`**) so whisper-cuda CMake cache stays warm (~1 min incremental dev). After CPU↔CUDA switches: **`npm run cargo -- clean -p whisper-rs-sys --manifest-path src-tauri/Cargo.toml`**. CUDA checks: **`npm run test:cargo-whisper-cuda`**. Env snapshot: **`npm run diagnose:build-env`**. Fresh clone / CI: **`npm run prebuild:gpu-cuda`** (see below). Fast bare cargo: **`JARVIS_SKIP_MODEL_FETCH=1 npm run cargo -- check …`**.

### Dev profiles (`--mac` / `--win` / `--audio`)

Platform-scoped dev builds trim overlap between Mac and Windows workflows. Default **`npm run tauri dev`** is unchanged (full stack, auto GPU, `src-tauri/target/`).

| Command | Host | Compile scope | Models fetched | Cargo target dir |
| --- | --- | --- | --- | --- |
| `npm run tauri dev` | any | full + auto GPU | all | `src-tauri/target/` |
| `npm run tauri:dev:mac` / `tauri dev --mac` | macOS only | full + metal | all | `.cache/cargo-target/mac-full-metal` |
| `npm run tauri:dev:win` / `tauri dev --win` | Windows only | full + cuda/vulkan | all | `.cache/cargo-target/win-full-*` |
| `npm run tauri:dev:mac:audio` / `tauri dev --mac --audio` | macOS | oww + whisper-metal (no LLM) | wake + tiny whisper | `.cache/cargo-target/mac-audio-metal` |
| `npm run tauri:dev:win:audio` / `tauri dev --win --audio` | Windows | oww + whisper GPU (no LLM) | wake + tiny whisper | `.cache/cargo-target/win-audio-*` |
| `npm run tauri:dev:cpu` / `tauri dev --cpu` | any | full, CPU whisper | all | `.cache/cargo-target/{mac\|win}-cpu-full` |

**`--mac` / `--win`** validate the host OS (no cross-compilation). **`--audio`** is for mic/wake/dictation iteration: skips `llm-local` compile and ~2 GB GGUF downloads. **`--audio` is dev-only** — release `tauri build` still expects full models in `tauri.conf.json`.

Aliases: `tauri:build:mac`, `tauri:build:win`. Compose with `--cpu` / `--gpu` as needed.

First run of a new profile uses a cold Cargo tree under `.cache/cargo-target/`; later runs are incremental within that profile and do not invalidate your default `target/` GPU cache.

### Windows GPU build env layers

Jarvis uses **four independent env sources** that do not always agree. Mixing them forces a **full 30–90+ min** rebuild of `whisper-rs-sys` and `llama-cpp-sys-2` CUDA trees.

| Layer | When | Typical `CMAKE_GENERATOR` |
| --- | --- | --- |
| **User/shell env** | Terminal, system env vars | Often `Visual Studio 17 2022` if set globally |
| **`config.local.toml`** | `npm install`, `npm run cargo`, rust-analyzer | **Ninja** or **NMake** when CUDA toolkit present (same as tauri); else `Visual Studio 17 2022` |
| **`npm run tauri *`** | Dev/build launcher | **NMake** or **Ninja** (forced by `detect.mjs` for CUDA) |
| **`npm run cargo`** with `whisper-cuda` / `llm-cuda` | CUDA cargo checks | Same as tauri CUDA path |

**Do not** set a global Windows user env var `CMAKE_GENERATOR=Visual Studio …` when using GPU tauri builds. The launcher **overrides** stale shell values to NMake/Ninja for CUDA, but bare `cargo check` in that same shell can still pick up the wrong generator and invalidate CMake cache.

**Workflow:** after **`npm install`**, `config.local.toml` and tauri CUDA share the same generator when the CUDA toolkit is installed — rust-analyzer and **`npm run tauri dev`** should not fight over CMake cache. Run **`npm run diagnose:build-env`** to compare shell vs tauri vs `config.local.toml`, check `ggml-cuda` artifacts, and list generator mismatches in `target/debug/build`.

#### CUDA CMake generator (Ninja vs NMake)

`detect.mjs` picks the generator for **`npm run tauri *`** and CUDA **`npm run cargo`** paths:

| Priority | Generator | Notes |
| --- | --- | --- |
| 1 | **`JARVIS_CMAKE_GENERATOR`** env | e.g. `NMake Makefiles` to force legacy NMake |
| 2 | **Ninja** | When `ninja.exe` is on `PATH`; sets `CMAKE_MAKE_PROGRAM` |
| 3 | **NMake Makefiles** | Fallback when Ninja is not installed |

Ninja parallelizes `.cu` compiles on Windows (often **~4–6×** faster than NMake in community benchmarks). Install: `winget install Ninja-build.Ninja`, reopen the terminal, verify with `where ninja`.

#### Compiler cache (ccache / sccache, optional)

When `ccache` or `sccache` is on `PATH`, `detect.mjs` sets `CMAKE_C_COMPILER_LAUNCHER`, `CMAKE_CXX_COMPILER_LAUNCHER`, and `CMAKE_CUDA_COMPILER_LAUNCHER` for whisper-rs-sys and llama-cpp-sys-2 CMake builds. Helps most after `cargo clean`, feature toggles, or dependency bumps — not the very first nvcc compile.

| Tool | Install | Cache dir (auto) |
| --- | --- | --- |
| **ccache** (preferred) | `winget install Ccache.Ccache` | `jarvis/.cache/ccache` (`CCACHE_DIR`) |
| **sccache** | `winget install Mozilla.sccache` | `jarvis/.cache/sccache` (`SCCACHE_DIR`) |

If neither is installed, the launcher **warns once** and continues without compiler caching. Verify with `where ccache` or `where sccache`. Startup log includes `compiler-cache=…` when enabled. Override with your own `CMAKE_*_COMPILER_LAUNCHER` env vars.

#### CUDA architecture pin (nvcc)

`npm run tauri *` and **`npm run cargo`** with `whisper-cuda` / `llm-cuda` set launcher env via `detect.mjs`:

| Variable | Default | Purpose |
| --- | --- | --- |
| `CMAKE_CUDA_ARCHITECTURES` | **`89`** (RTX 40 default) | Single-arch nvcc build — much faster than multi-gen defaults |
| `CMAKE_BUILD_PARALLEL_LEVEL` | CPU core count | Parallel CMake jobs for whisper-rs-sys + llama-cpp-sys-2 |
| `WHISPER_DONT_GENERATE_BINDINGS` | **`1`** on Linux/macOS CUDA paths only | Windows runs bindgen (bundled `bindings.rs` is Linux glibc → E0080) |

Override arch for other GPUs with **`JARVIS_CUDA_ARCH`** (e.g. `86` for RTX 30 Ampere). At startup the launcher logs `CUDA build profile: arch=…; generator=…; parallel=…; CUDA_PATH=…`.

| NVIDIA GPU | Compute | `JARVIS_CUDA_ARCH` |
| --- | --- | --- |
| RTX 4090 / 4080 / 4070 / 4060 (Ada) | 8.9 | `89` (default) |
| RTX 3090 / 3080 / 3070 (Ampere) | 8.6 | `86` |
| RTX 2080 / 2070 (Turing) | 7.5 | `75` |

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
- The launcher shows a **GPU build progress bar** on stderr (~5s updates) with phase/debug lines on stalls; disable with `JARVIS_GPU_BUILD_PROGRESS=0`.
- If bindgen failed earlier, clean before retry: `npm run cargo -- clean -p whisper-rs-sys --manifest-path src-tauri/Cargo.toml`

### Whisper GPU backend auto-selection

`scripts/whisper-gpu/run-tauri.mjs` chooses one backend per artifact:

- macOS host -> `whisper-metal`
- Windows/Linux + NVIDIA GPU + CUDA toolkit -> `whisper-cuda`
- Other GPUs (or NVIDIA without CUDA) + Vulkan SDK -> `whisper-vulkan`
- Missing toolchains -> CPU-only Whisper (`whisper` GPU features off; warning logged)

Install **CUDA** or **Vulkan SDK** manually (see NVIDIA / Khronos docs); set `CUDA_PATH` or `VULKAN_SDK` if not auto-discovered.

**Launcher flags:** `npm run tauri:dev -- --cpu` or **`npm run tauri:dev:cpu`** for CPU-only; **`npm run tauri:dev:gpu`** for explicit GPU auto-select.

**CUDA dev profile:** when the backend is `whisper-cuda`, `npm run tauri dev` (and `:gpu`) builds a **`--release` native binary** while Vite HMR stays in dev mode — avoids MSVC debug stack overrun during Whisper CUDA preload (`docs/bugs/BUG-debug-cuda-whisper-stack-buffer-overrun.md`). Opt into debug native with `JARVIS_GPU_DEV_DEBUG=1`. During long first CUDA compiles, stderr shows a **GPU build progress bar** (~5s); disable with `JARVIS_GPU_BUILD_PROGRESS=0`.

**Unified build env:** `scripts/build-environment.mjs` (`resolveBuildEnvironment`, `prepareGpuNativeBuild`) is the single seam used by `run-tauri.mjs`, `cargo-win-env.mjs`, and `sync-cargo-win-env.mjs`. Run **`npm run diagnose:build-env`** to compare shell vs tauri vs `config.local.toml`.

**Rebuild loop:** mixing a global shell `CMAKE_GENERATOR=Visual Studio …` with GPU tauri builds, or switching Ninja ↔ NMake without cleaning, forces a full `whisper-rs-sys` / `llama-cpp-sys-2` rebuild. Stay on one workflow until `ggml-cuda.lib` exists, then incremental dev is typically ~1–2 minutes.

#### Clean CUDA build benchmark (Windows, arch `89`)

Measured on **RTX 40-class GPU**, **CUDA 13.2**, **VS 2022 MSVC**, **`CMAKE_CUDA_ARCHITECTURES=89`**, clean `target/debug/build/*/out/build` for both `-sys` crates then full `cargo` GPU check (`whisper-cuda` + `llm-cuda`):

| Generator | Wall time (whisper-rs-sys + llama-cpp-sys-2 CUDA) | Notes |
| --- | --- | --- |
| **NMake Makefiles** | ~45–90 min typical | Serial `.cu` compiles; `JARVIS_CMAKE_GENERATOR=NMake Makefiles` |
| **Ninja** | ~20–45 min typical | `winget install Ninja-build.Ninja`; parallel nvcc via `CMAKE_BUILD_PARALLEL_LEVEL` |

Reproduce (from `jarvis/`):

```powershell
# NMake baseline
$env:JARVIS_CMAKE_GENERATOR = "NMake Makefiles"
npm run cargo -- clean
Remove-Item -Recurse -Force src-tauri\target\debug\build\whisper-rs-sys-*\out\build, src-tauri\target\debug\build\llama-cpp-sys-2-*\out\build -ErrorAction SilentlyContinue
Measure-Command { npm run cargo -- check --manifest-path src-tauri/Cargo.toml --features llm-local,whisper-cuda,llm-cuda,oww }

# Ninja (default when ninja.exe on PATH)
Remove-Item Env:JARVIS_CMAKE_GENERATOR -ErrorAction SilentlyContinue
npm run cargo -- clean
Remove-Item -Recurse -Force src-tauri\target\debug\build\whisper-rs-sys-*\out\build, src-tauri\target\debug\build\llama-cpp-sys-2-*\out\build -ErrorAction SilentlyContinue
Measure-Command { npm run cargo -- check --manifest-path src-tauri/Cargo.toml --features llm-local,whisper-cuda,llm-cuda,oww }
```

Times vary by CPU core count, disk, and driver; treat as order-of-magnitude guidance.

#### GPU CUDA prebuild cache (CI / fresh clones)

First `npm run tauri dev` with `whisper-cuda` + `llm-cuda` compiles **two** full ggml-cuda trees. **`npm run prebuild:gpu-cuda`** runs standalone CMake **Release** builds once (arch **`89`** default, same generator/nvcc env as tauri) and stores artifacts under **`jarvis/.cache/gpu-prebuild/<arch>/`**.

| Step | Command | Purpose |
| --- | --- | --- |
| 1 | `npm run cargo -- fetch --manifest-path src-tauri/Cargo.toml` | Ensure `whisper-rs-sys` / `llama-cpp-sys-2` sources are in the local cargo registry |
| 2 | `npm run prebuild:gpu-cuda` | Build whisper.cpp + llama.cpp CUDA into `.cache/gpu-prebuild/89/` (20–45+ min first time) |
| 3 | `npm run tauri:dev:gpu` | Launcher seeds warm cache into cargo `-sys` `out/build` when dirs exist; incremental link/configure |

**CI workflow:** run step 2 on a GPU builder agent, upload **`jarvis/.cache/gpu-prebuild/`** as a cache artifact (key: `gpu-prebuild-${{ hashFiles('jarvis/src-tauri/Cargo.lock') }}-89`). Restore before step 3 on PR agents. Pair with **ccache/sccache** (above) so nvcc object files hit compiler cache even when CMake out dirs differ.

**Fresh clone (local):**

```powershell
cd jarvis
npm ci
npm run cargo -- fetch --manifest-path src-tauri/Cargo.toml
npm run prebuild:gpu-cuda
npm run tauri:dev:gpu
```

Options: `--force` rebuild cache; `--arch 86` for Ampere; `--skip-seed` cmake-only. Env: `JARVIS_GPU_PREBUILD_ROOT`, `JARVIS_GPU_PREBUILD_WARM=1` when cache matches `Cargo.lock` + generator. Check status: **`npm run diagnose:build-env`**.

The wrapper runs the Tauri CLI via `node node_modules/@tauri-apps/cli/tauri.js` (not `tauri.cmd`) after GPU detection and bindgen preflight on Windows.

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

- **HUD (overlay):** global shortcut (default **Ctrl+Shift+J**, configurable in Settings → Hotkeys) opens and closes the HUD.
- **Editor window:** standard **Tab** / **Shift+Tab** focus order; form fields and buttons have no separate global chord beyond OS defaults. After changing the hotkey in Settings, the new combo applies immediately after a successful save.

### Migrations

SQLite schema changes are **additive** migrations run at startup (e.g. `sort_order` on `command_nodes`). See `**src-tauri/MIGRATIONS.md`** for the log and idempotency notes. Copy existing user DBs forward without destructive resets for Phase 3 changes.

### Tests and coverage

From `jarvis/`: `npm run test` runs Vitest. Coverage thresholds (**≥70%** lines on `editorStore`, `NodeForm.logic`) are enforced when you run `npm run test:coverage`.

## Phase 2 manual verification (Windows)

Use this gate before calling a Phase 2 build releasable:

1. Trigger fuzzy phrase (typo) and confirm intended command still matches/executes.
2. Run a multi-action chain (`OpenApp` -> `Wait` -> `OpenUrl`) and confirm strict order.
3. Run at least one command with `Speak`; confirm audible output or controlled Piper-missing error.
4. Run the `subprompt test` voice command, answer with a topic (for example `rust`), and confirm browser opens GitHub search results for that topic.
5. Re-run `SubPrompt` and let it timeout (or cancel); confirm safe terminal HUD phase, no crash/deadlock.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

