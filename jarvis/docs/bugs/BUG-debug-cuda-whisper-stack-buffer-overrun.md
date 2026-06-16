# BUG: Debug `tauri dev` crash — `STATUS_STACK_BUFFER_OVERRUN` during Whisper CUDA preload

| Field | Value |
| --- | --- |
| **ID** | `BUG-debug-cuda-whisper-stack-buffer-overrun` |
| **Status** | **Partially mitigated** (2026-06-13) — stack + native build hardening shipped; pending 10× `tauri:dev` GPU validation |
| **Severity** | P1 for GPU + local-LLM dev workflow on Windows |
| **First seen** | 2026-06-12 |
| **Platform** | Windows 10/11, MSVC **debug** profile (`cargo run` / `tauri dev`) |
| **GPU** | NVIDIA RTX 4060, CUDA 13.2, arch **sm_89** |
| **Features** | `llm-local`, `whisper-cuda`, `llm-cuda`, `oww` |

## Summary

`jarvis.exe` intermittently terminates during startup with Windows exit code **`0xc0000409`** (`STATUS_STACK_BUFFER_OVERRUN`) while the **Whisper CUDA** speech model is loading on the background `whisper-preload` thread. Tier 2 LLM router warmup is **not** required to reproduce. Disabling **`local_whisper_use_gpu`** in settings stabilizes debug launches; release builds are expected to be more reliable but are not fully validated here.

This blocks the desired dev configuration: **CUDA Whisper STT + local LLM (`llm-cuda`)** under `npm run tauri:dev`.

## Symptoms

Typical log sequence immediately before exit:

```text
[INFO  jarvis_lib] whisper: loading STT model (debug MSVC builds can take several minutes)
whisper_init_from_file_with_params_no_state: loading model from '...\ggml-tiny.en.bin'
whisper_init_with_params_no_state: use gpu    = 1
whisper_init_with_params_no_state: flash attn = 1
ggml_cuda_init: found 1 CUDA devices ...
whisper_model_load: loading model
[INFO  jarvis_lib::audio::wake::thread] wake: thread started backend=oww
error: process didn't exit successfully: `target\debug\jarvis.exe` (exit code: 0xc0000409, STATUS_STACK_BUFFER_OVERRUN)
```

Alternate exit code observed in related runs: **`0x80000003`** (`STATUS_BREAKPOINT`) — documented in `stt.rs` as a possible Whisper GPU init failure mode on Windows.

Crash is **timing-dependent**: some runs survive through app-index scan (~12s); others die within the same second as `whisper_model_load`.

## Reproduction

### Prerequisites

- Windows dev machine with NVIDIA GPU + CUDA toolkit
- `npm run tauri:dev` from `jarvis/` (features `llm-local,whisper-cuda,llm-cuda,oww`)
- `ggml-tiny.en.bin` present under `jarvis/src-tauri/resources/`
- SQLite settings (`%APPDATA%\com.jarvis.app\jarvis.db`):

| Setting | Value for repro |
| --- | --- |
| `stt_provider` | `local` |
| `local_whisper_use_gpu` | `1` |
| `wake_engine` | `oww` |
| `llm_router_tier2_enabled` | `0` (ruled out as required) |
| `llm_router_warmup_on_launch` | `0` (ruled out as required) |

### Steps

1. Confirm settings above (or enable GPU whisper in Settings UI).
2. Run `npm run tauri:dev`.
3. Observe crash during or shortly after `whisper_model_load: loading model`.

### Control (stable)

Set `local_whisper_use_gpu` to `0` → Whisper loads on CPU → process remains alive 15s+ in manual testing.

## Investigation timeline

| Hypothesis | Result |
| --- | --- |
| Missing ccache / build env | Ruled out — unrelated to runtime crash |
| Concurrent **llama-cpp CUDA router warmup** + Whisper CUDA | Partially relevant when Tier 2 + warmup on launch enabled; **not required** — crash persists with both disabled |
| **Wake OWW** thread (`ort` ONNX) | Coincident in logs; OWW init completes before `wake: thread started`; not primary cause |
| **Whisper CUDA preload** in debug MSVC build | **Confirmed** — CPU GPU setting removes crash |

## Root cause (current understanding)

1. **Trigger:** `spawn_whisper_preload_on_launch` starts a background thread that calls `load_whisper_context_serialized(..., use_gpu: true)` when `local_whisper_use_gpu` is enabled and the binary is built with `whisper-cuda`.

2. **Failure site:** Native **whisper.cpp / ggml-cuda** code path during `whisper_model_load` (weight load to GPU), not Rust panics. Windows reports `/GS` stack-buffer security failure (`STATUS_STACK_BUFFER_OVERRUN`).

3. **Debug amplification:**
   - `dev` profile → unoptimized ggml → very deep C++ call stacks during model parse and CUDA setup.
   - MSVC debug + `/GS` stack cookies surface overruns that optimized release builds may not trip.
   - `flash_attn(true)` is enabled for `whisper-cuda` builds, adding more native work during init.

4. **Concurrency at startup** (same `setup` hook, overlapping threads):
   - `whisper-preload` — 32 MiB stack (`WHISPER_LOADER_STACK`), CUDA model load
   - `jarvis-wake` — default thread stack, OWW ONNX + cpal mic
   - `app-index` scan — filesystem only
   - Main thread — tray, hotkeys, Tauri window

   Whisper load is serialized **within** Whisper via `WHISPER_LOAD_GATE`, but there is **no cross-subsystem GPU init gate** between whisper-rs ggml-cuda and llama-cpp-cuda when router warmup is enabled.

## Affected code

| Symbol / area | File | Role |
| --- | --- | --- |
| `spawn_startup_sequence` | `jarvis/src-tauri/src/gpu_startup.rs` | Ordered launch: whisper → router → wake |
| `with_ggml_cuda_init` | `jarvis/src-tauri/src/gpu_startup.rs` | Cross-subsystem CUDA init gate |
| `run_whisper_preload_at_launch` | `jarvis/src-tauri/src/lib.rs` | Whisper preload + `whisper-model-warmup` emit |
| `warmup_whisper_model_blocking` | `jarvis/src-tauri/src/lib.rs` | Calls serialized Whisper load |
| `WHISPER_LOADER_STACK` | `jarvis/src-tauri/src/gpu_startup.rs` | 32 MiB stack for coordinator / loader threads |
| `spawn_whisper_loader_thread` | `jarvis/src-tauri/src/gpu_startup.rs` | Large-stack spawn for all whisper native loads |
| `load_whisper_context` | `jarvis/src-tauri/src/audio/stt.rs` | `use_gpu`, release-only `flash_attn`, GPU→CPU fallback |
| `load_whisper_context_serialized` | `jarvis/src-tauri/src/audio/stt.rs` | `WHISPER_LOAD_GATE` mutex |
| `start_wake_from_settings` | `jarvis/src-tauri/src/lib.rs` | Wake supervisor install (deferred when GPU whisper preloads) |
| `run_router_warmup_at_launch_blocking` | `jarvis/src-tauri/src/llm/tauri_cmds.rs` | Blocking router warmup for launch coordinator |
| `infer_router_completion` | `jarvis/src-tauri/src/llm/local.rs` | llama-cpp CUDA behind `with_ggml_cuda_init` |

## Evidence

- **User DB** (`com.jarvis.app/jarvis.db`): with `llm_router_tier2_enabled=0` and `llm_router_warmup_on_launch=0`, crash still occurs when `local_whisper_use_gpu=1`.
- **Manual A/B** (2026-06-14): `local_whisper_use_gpu=0` → `target/debug/jarvis.exe` alive ≥15s; `=1` → intermittent crash at whisper load window.

## Fix (shipped 2026-06-13, extended same day)

Implemented in Rust startup path — no release-only or disable-GPU workarounds:

1. **`gpu_startup.rs`** — `GGML_CUDA_INIT_GATE` serializes ggml-CUDA init across whisper-rs and llama-cpp; `spawn_startup_sequence` runs whisper preload → deferred app-index scan → router warmup → wake on one `jarvis-gpu-startup` thread (32 MiB stack when Whisper loads); `spawn_whisper_loader_thread` for all whisper native loads.
2. **`stt.rs`** — `flash_attn` enabled only in **release** `whisper-cuda` builds (`whisper_flash_attn_enabled()`); GPU `WhisperContext::new` wrapped with `with_ggml_cuda_init`; manual GPU load test on loader thread.
3. **`local.rs`** — router GPU infer wrapped with `with_ggml_cuda_init`; `ROUTER_LOADER_STACK` aliases `WHISPER_LOADER_STACK`.
4. **`lib.rs` setup** — defers wake and app-index background scan when GPU Whisper preload is scheduled; startup diagnostic log for `defer_wake` / `whisper_use_gpu`.
5. **`.cargo/config.toml`** — Windows `rustflags` `/STACK:16777216` (16 MiB default thread stack for `jarvis.exe`).
6. **`detect.mjs`** — `CMAKE_BUILD_TYPE=Release` for CUDA builds (overrides whisper-rs-sys `RelWithDebInfo` in debug Rust builds; requires `cargo clean -p whisper-rs-sys` once).

Manual validation: 10× `npm run tauri:dev` with `local_whisper_use_gpu=1` (and full tier2+warmup config) — confirm no `0xc0000409`.

### Validation checklist (post stack + CMAKE hardening)

1. One-time native rebuild after `CMAKE_BUILD_TYPE=Release` (use `npm run tauri:dev`, not bare `cargo`):
   ```powershell
   cd jarvis
   npm run cargo -- clean -p whisper-rs-sys --manifest-path src-tauri/Cargo.toml
   npm run tauri:dev
   ```
2. Startup log: `gpu-startup: whisper_preload=true whisper_use_gpu=true defer_wake=true`.
3. whisper.cpp: `flash attn = 0` in debug.
4. `wake: thread started` **after** `whisper-model-warmup`, not during `whisper_model_load`.
5. GPU isolation: `npm run cargo -- test --manifest-path src-tauri/Cargo.toml --features whisper-cuda,llm-local,oww manual_load_whisper_tiny_model_gpu -- --ignored --nocapture` (via `npm run tauri:dev` env if bare cargo fails).
6. Repeat launch 10× for stability.

**Agent validation (2026-06-14):** `detect.test.mjs` passes (`CMAKE_BUILD_TYPE=Release`). Full `tauri:dev` GPU runtime loop pending CUDA rebuild on dev machine.

If still failing after rebuild: WinDbg frame above `__report_gsfailure`, or `npm run tauri:dev:release`, or evaluate whisper-rs 0.16 upgrade.

## Workarounds (historical — pre-fix)

| Need | Workaround |
| --- | --- |
| Stable **debug** UI work | Disable **local Whisper GPU** in Settings; keep CUDA features compiled for other paths |
| **CUDA STT** required | Use **release** build (`npm run tauri build`) and run the release binary |
| Router + Whisper GPU both on launch | Disable **router warmup on launch**; avoid parallel CUDA init (reduces but does not eliminate whisper-only crash) |

## Proposed fixes (superseded by shipped fix)

Priority order for implementation:

1. **Serialize GPU warmup** — done via `spawn_startup_sequence` + `GGML_CUDA_INIT_GATE`.
2. **Defer wake thread** — done when GPU whisper preload runs at launch.
3. **Disable `flash_attn` under `debug_assertions`** — done in `whisper_flash_attn_enabled()`.
4. **Increase `WHISPER_LOADER_STACK`** — done (32 MiB + `/STACK:16MiB` linker flag).
5. **Auto-fallback** — existing GPU→CPU retry in `load_whisper_context` unchanged.
6. **Document** release-as-GPU-dev workflow — not required once debug path is stable.

## Related docs

- GPU **build** time (not this runtime bug): `.cursor/plans/gpu_build_optimization_10f84a66.plan.md`
- Whisper Windows bindgen / CMake: `jarvis/README.md` Prerequisites

## Codegraph symbols

Search these names when tracing call paths:

`spawn_startup_sequence`, `spawn_whisper_loader_thread`, `with_ggml_cuda_init`, `defer_wake_for_gpu_whisper`, `run_whisper_preload_at_launch`, `warmup_whisper_model_blocking`, `load_whisper_context_serialized`, `load_whisper_context`, `whisper_flash_attn_enabled`, `WHISPER_LOAD_GATE`, `WHISPER_LOADER_STACK`, `run_router_warmup_at_launch_blocking`, `start_wake_from_settings`
