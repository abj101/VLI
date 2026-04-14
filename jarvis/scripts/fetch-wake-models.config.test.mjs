import { describe, it, expect } from "vitest";
import { oww, porcupine } from "./fetch-wake-models.config.mjs";

describe("fetch-wake-models.config", () => {
  it("lists Porcupine Windows bundle files", () => {
    expect(porcupine.files.map(([, name]) => name)).toEqual(
      expect.arrayContaining([
        "libpv_porcupine.dll",
        "porcupine_params.pv",
        "porcupine_windows.ppn",
      ]),
    );
  });

  it("lists OpenWakeWord ONNX files expected by oww.rs", () => {
    expect(oww.files).toEqual([
      "melspectrogram.onnx",
      "embedding_model.onnx",
      "hey_jarvis_v0.1.onnx",
    ]);
  });
});
