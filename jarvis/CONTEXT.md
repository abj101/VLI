# Jarvis — domain glossary

Terms used in build-system architecture reviews and ADRs. Prefer these names over file or script identifiers.

## Build channel

One of three entry points that resolve the same build environment through `resolveBuildEnvironment()`:

| Channel | Script | Role |
|---------|--------|------|
| `tauri` | `scripts/whisper-gpu/run-tauri.mjs` | Dev/build launcher — full CUDA, bindgen, prebuild env |
| `cargo` | `scripts/cargo-win-env.mjs` | Bare `cargo` / rust-analyzer — full path when CUDA features requested |
| `sync` | `scripts/sync-cargo-win-env.mjs` | Writes `src-tauri/.cargo/config.local.toml` from discovery only |
| `prebuild` | `scripts/prebuild-gpu-cuda.mjs` | One-shot GPU cache populate — CUDA env via same resolver, no bindgen |

The `sync` channel always uses `needsCuda: true` on Windows so `CMAKE_GENERATOR` in `config.local.toml` matches the Ninja/NMake generator used by the GPU path, preventing CMake cache mismatches when switching CPU ↔ CUDA features.

## Build environment snapshot

Frozen output of `resolveBuildEnvironment()`: `{ env, cudaBuildEnv, generator, gpuPrebuild, discovered }`.

Callers must pass `env` to spawned children; do not read `process.env` after resolve (toolkit discovery may mutate globals on non-`sync` channels).

## GPU native sys-crate seam

`scripts/gpu-native-sys.mjs` — CMake cache layout, validation (`isCmakeCacheWellFormed`), and purge for `whisper-rs-sys` / `llama-cpp-sys-2` cargo `out/build` trees. Consumed by `gpu-prebuild.mjs`, `preflight.mjs`, and `prepareGpuNativeBuild()`.

## GPU prebuild cache

Warm CMake build trees under `jarvis/.cache/gpu-prebuild/<arch>/` for `whisper-rs-sys` and `llama-cpp-sys-2`. Populated by `npm run prebuild:gpu-cuda`; consumed by `prepareGpuNativeBuild()` when `JARVIS_GPU_PREBUILD_WARM=1`.

## Dictation session

Voice-dictation period: STT partials stream into the focused text field without HUD focus steal. One session owns rolling decode state (`last_stt_text`), on-screen segment tracking (`segment_injected`), and cumulative typed text (`session_typed`).

## Transcript reconcile

Deep module (`dictation/reconcile.rs`): `TranscriptReconciler::apply_with_kind(partial, kind) → ReconcileResult`. Owns segment + `ScreenModel` state. Policy: `growth` → char-prefix delta; `revision` → overlap merge, then segment replace, then full-span fallback (never append); new utterance → overlap merge; `silence_reset` → clear rolling STT only.

## Transcript partial kind

STT event tag (`audio/transcript_event.rs`): `growth`, `revision`, `silence_reset`. Carried on `TranscriptUpdate.kind`; dictation reconcile consumes it directly.

## Dictation session controller

Lifecycle module (`dictation/session.rs`): start/stop/toggle, flush on stop, HUD collision guard. Holds `TranscriptReconciler` per session.

## Text injector seam

Adapter turning reconcile deltas into OS keystrokes. `SystemTextInjector` in production — batched `SendInput` backspaces (32/burst), brief settle delay before large retypes; `MockTextInjector` / `VirtualScreen` in tests. Invariant: dictation only mutates field tail; `ScreenModel` clamps backspace count to typed screen length.
