# JARVIS

JARVIS is a local-first voice assistant built with Tauri, React, and TypeScript. Speech recognition and local language models run through prebuilt Whisper and llama.cpp server sidecars.

## Prerequisites

### Windows

- Rust stable and Node.js LTS
- Visual Studio 2022 Build Tools or Visual Studio with the **Desktop development with C++** workload and Windows SDK
- For CUDA sidecars: NVIDIA drivers and `nvidia-smi`; the fetch script selects the CUDA 12.4 runtime assets when available

Rust's MSVC target uses the standard Visual Studio toolchain discovery. CMake, LLVM/libclang, Ninja, CUDA Toolkit, and bindgen are not needed to compile the Rust application.

### macOS

- Rust stable, Node.js LTS, and Xcode command line tools (`xcode-select --install`)
- `whisper-server` and `llama-server` available on `PATH` (for example, installed with Homebrew)
- Allow microphone access for `com.jarvis.app`; dev builds use the app's microphone entitlements

## Fetch models and runtimes

From `jarvis/`, install dependencies and start the app:

```sh
npm install
npm run tauri:dev
```

The npm `predev` and `prebuild` hooks fetch the wake-word models, Whisper model weights, LLM models, and pinned server runtimes before Vite starts or the frontend builds. On Windows the fetch script selects CUDA 12.4 sidecar archives when `nvidia-smi` is available; otherwise it fetches CPU builds. Downloaded files are placed under `src-tauri/resources/` and are not committed. You can also fetch them explicitly with `npm run fetch-models`.

To fetch only selected model groups, use `node scripts/fetch-models.mjs --sets wake,whisper-tiny,llm`. The sidecars are included by Tauri bundling from `src-tauri/resources/bin/`.

## Run and build

From `jarvis/`:

```sh
npm run tauri:dev
npm run tauri:build
```

On macOS, the runtime check expects both server executables on `PATH`.

Other useful commands:

- `npm run lint` — ESLint
- `npm test` — frontend tests and composer fixtures
- `npm run build` — TypeScript check and Vite production build
- `cargo test --manifest-path src-tauri/Cargo.toml` — Rust tests

Run raw Cargo commands such as `cargo test` or `cargo build` from `jarvis/src-tauri/` so Cargo discovers `.cargo/config.toml` and uses the macOS linker helper and signing setup.

## Piper voice output

The optional `Speak` action looks for Piper and its voice model using `JARVIS_PIPER_BIN` and `JARVIS_PIPER_MODEL` (or `PIPER_BIN` and `PIPER_MODEL`). Bundled fallbacks live in `src-tauri/resources/piper/`.

## License

MIT. See [`../LICENSE`](../LICENSE).
