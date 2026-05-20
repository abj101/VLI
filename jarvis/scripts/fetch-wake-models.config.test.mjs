import { describe, expect, it } from "vitest";
import { oww } from "./fetch-wake-models.config.mjs";

describe("fetch-wake-models.config", () => {
  it("lists OpenWakeWord bundle files", () => {
    expect(oww.files).toEqual([
      "melspectrogram.onnx",
      "embedding_model.onnx",
      "hey_jarvis_v0.1.onnx",
    ]);
  });
});
