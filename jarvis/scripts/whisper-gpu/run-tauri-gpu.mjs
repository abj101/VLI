#!/usr/bin/env node
/** Tauri dev/build with GPU auto-select (CUDA / Vulkan / Metal). First CUDA build is slow. */
process.env.WHISPER_GPU_BACKEND = "auto";
await import("./run-tauri.mjs");
