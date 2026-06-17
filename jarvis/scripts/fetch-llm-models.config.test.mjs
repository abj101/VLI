import { describe, expect, it } from "vitest";
import { llmModels } from "./fetch-llm-models.config.mjs";

describe("fetch-llm-models.config", () => {
  it("lists router and composer GGUF filenames", () => {
    expect(llmModels.map((m) => m.file)).toEqual([
      "qwen2.5-0.5b-instruct-q4_k_m.gguf",
      "qwen2.5-3b-instruct-q4_k_m.gguf",
    ]);
  });
});
