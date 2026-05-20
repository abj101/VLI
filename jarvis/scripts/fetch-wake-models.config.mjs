/**
 * URLs and paths for wake-model fetch (OpenWakeWord).
 * Keep in sync with `audio/wake/oww.rs` ONNX filenames and `scripts/download-oww-model.ps1`.
 */
export const oww = {
  destSubdir: "oww",
  base: "https://github.com/dscripka/openWakeWord/releases/download/v0.5.1",
  /** Filenames under `resources/oww/`; must match `oww.rs` MELSPEC_ONNX, etc. */
  files: [
    "melspectrogram.onnx",
    "embedding_model.onnx",
    "hey_jarvis_v0.1.onnx",
  ],
};
