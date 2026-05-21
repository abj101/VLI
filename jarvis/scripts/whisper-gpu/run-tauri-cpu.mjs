#!/usr/bin/env node
/** CPU-only Tauri dev/build — skips whisper-cuda/vulkan/metal auto-select. */
process.env.WHISPER_GPU_BACKEND = "none";
await import("./run-tauri.mjs");
