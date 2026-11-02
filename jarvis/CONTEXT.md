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

Voice-dictation period using the shared HUD session counter. Active dictation shows only a microphone waveform beside the caret. On Windows, starting without an editable field opens a centered main HUD with a waiting prompt and pauses microphone capture; focusing a field morphs that surface into the caret orb. Initial waiting dismisses after ten seconds. Losing focus later pauses capture and retains untyped text until a field returns, without an automatic timeout. Partial transcripts never touch the focused field. Each final utterance is inserted once, without deleting existing text.

## Dictation utterance

Local and remote STT retain up to 28 seconds of audio for a final decode. Preview decodes use at most the latest four seconds. Toggle dictation finalizes after 700 ms of silence; push-to-talk finalizes on release. Reaching the cap finalizes either mode and starts a fresh utterance.

## Transcript partial kind

STT event tag (`audio/transcript_event.rs`): `growth`, `revision`, `silence_reset`. Carried on `TranscriptUpdate.kind` for command recognition. Dictation uses `is_final` to distinguish previews from insertion.

## Dictation session controller

Lifecycle module (`dictation/session.rs`): active session, inserted text tail for spacing and Whisper context, utterance count, and email/general context captured from the focused app at session start. Slow cleanup and insertion run outside its mutex.

## Dictation insertion

Final text is pasted through the existing clipboard and key-chord helpers, then the previous text clipboard is restored. Batched UTF-16 keyboard input is the fallback. Dictation never backspaces or rewrites the focused field.

## Dictation cleanup

S1-mini by Superwhisper runs in a separate resident llama-server sidecar. Each final utterance uses the model's exact normalization prompt, with an 800 ms timeout and raw-text fallback when unavailable or invalid. Cleanup style is configurable; personal vocabulary feeds only the Whisper dictation prompt.
