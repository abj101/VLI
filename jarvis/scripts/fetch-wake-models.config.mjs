/**
 * URLs and paths for wake-model fetch (Porcupine + OpenWakeWord).
 * Keep in sync with `audio/wake/oww.rs` ONNX filenames and `audio/wake/porcupine`.
 */
export const porcupine = {
  destSubdir: "porcupine",
  base: "https://raw.githubusercontent.com/Picovoice/porcupine/master",
  /** [repo-relative path, local filename] */
  files: [
    ["lib/windows/amd64/libpv_porcupine.dll", "libpv_porcupine.dll"],
    ["lib/common/porcupine_params.pv", "porcupine_params.pv"],
    [
      "resources/keyword_files/windows/porcupine_windows.ppn",
      "porcupine_windows.ppn",
    ],
  ],
};

/** Matches `openWakeWord` release used by `scripts/download-oww-model.ps1`. */
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
