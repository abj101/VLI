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
