# VLI

**Voice-driven desktop companion** — built around a simple idea: your computer should feel less like navigating menus and more like actually talking to it. Think responsive overlays, quick actions, and a desktop that reacts fast enough to stay out of your way.

This project is **open source** and very much a work in progress. APIs, architecture, and UX will change as the project grows. Releases are snapshots of where the system is right now, not guarantees of long-term stability.

## What this is

VLI is a local-first experiment combining **voice interaction**, **desktop overlays**, and **automation** into one system. Speak naturally, transcribe locally, match intent, and trigger actions directly on your machine.

The focus is keeping things responsive and personal without routing every microphone input through someone else’s servers.

Not every implementation detail or roadmap item lives in this README. The deeper technical docs stay closer to the actual codebase.

* **Application source:** [`jarvis/`](./jarvis/)
* **Developer setup (models, scripts, tooling):** [`jarvis/README.md`](./jarvis/README.md)

## Tech stack

| Layer                  | Choices                                                                                                   |
| ---------------------- | --------------------------------------------------------------------------------------------------------- |
| **Desktop shell**      | [Tauri](https://tauri.app/) for a lightweight native desktop host with Rust under the hood                |
| **Frontend**           | [React](https://react.dev/) + [TypeScript](https://www.typescriptlang.org/) + [Vite](https://vitejs.dev/) |
| **Speech recognition** | Local Whisper-based ASR through [`whisper-rs`](https://github.com/tazz4843/whisper-rs) / whisper.cpp      |
| **Wake words**         | Modular wake-word backends, including ONNX-compatible pipelines                                           |
| **Voice output**       | [Piper](https://github.com/rhasspy/piper) TTS                                                             |
| **Persistence**        | [SQLite](https://sqlite.org/) for commands, settings, and automation data                                 |

A lot of the interesting work here sits at the intersection of **real-time audio**, **systems programming**, **local ML**, and trying to make a desktop UI feel alive without becoming distracting.

## Custom wake language

Default wake words are fine, but they rarely feel natural. Part of this project involved retraining wake-word models around phrases that actually feel good to say during normal use.

The goal is flexibility — owning the interaction layer instead of being locked into whatever keyword another platform decided users should say.

Most of the deeper details around datasets, thresholds, exports, and tuning live in the code and technical notes rather than this README.

## Challenges

* **Latency matters:** mic → wake word → transcription → intent → action has to happen fast enough to feel immediate.
* **Wake-word tuning is messy:** tuning the  parameters for an accurate vs precise model.
* **Cross-platform audio is painful:** drivers, permissions, GPUs, and installers all behave differently.
* **Local-first tradeoffs are real:** keeping everything on-device is great until users expect cloud-level convenience with zero setup.

A lot of this project is ongoing iteration and tuning rather than “solving” things once.

## Contributing & license

Pull requests, issues, and experiments are welcome. If you’re planning major architectural changes, opening a discussion first is appreciated.

Licensed under the **MIT License** — see [`LICENSE`](./LICENSE).

## Status

**Actively in development.** Some features are incomplete, experimental, or hidden behind flags while systems get reworked.

---
