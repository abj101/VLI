import { describe, expect, it } from "vitest";

import { resolveBackend } from "./detect.mjs";

describe("resolveBackend", () => {
  const prev = process.env.WHISPER_GPU_BACKEND;

  it("honors WHISPER_GPU_BACKEND=none override", () => {
    process.env.WHISPER_GPU_BACKEND = "none";
    const selected = resolveBackend();
    expect(selected.backend).toBe("none");
    expect(selected.forced).toBe(true);
    if (prev === undefined) {
      delete process.env.WHISPER_GPU_BACKEND;
    } else {
      process.env.WHISPER_GPU_BACKEND = prev;
    }
  });

  it("honors WHISPER_GPU_BACKEND=cuda override when forced", () => {
    process.env.WHISPER_GPU_BACKEND = "cuda";
    const selected = resolveBackend();
    expect(selected.backend).toBe("cuda");
    expect(selected.forced).toBe(true);
    if (prev === undefined) {
      delete process.env.WHISPER_GPU_BACKEND;
    } else {
      process.env.WHISPER_GPU_BACKEND = prev;
    }
  });
});
