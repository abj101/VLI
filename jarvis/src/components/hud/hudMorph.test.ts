import { describe, expect, it } from "vitest";
import { dictationMorphGeometry } from "./hudMorph";

describe("dictation morph geometry", () => {
  it("preserves the exact command outline on the first frame", () => {
    expect(dictationMorphGeometry(480, 172, 480)).toEqual({ inset: 0, radius: 12 });
    const next = dictationMorphGeometry(479, 172, 480);
    expect(next.inset).toBeLessThan(0.01);
    expect(Math.abs(next.radius - 12)).toBeLessThan(0.2);
  });

  it("finishes as a circle at every supported caret size", () => {
    for (const size of [20, 28, 36]) {
      expect(dictationMorphGeometry(size, size, 480)).toEqual({ inset: 2, radius: size / 2 - 2 });
      expect(dictationMorphGeometry(size, size, size)).toEqual({ inset: 2, radius: size / 2 - 2 });
    }
  });

  it("adds edge clearance continuously as the native bounds shrink", () => {
    let previous = 0;
    for (let width = 480; width >= 28; width--) {
      const geometry = dictationMorphGeometry(width, Math.min(width, 172), 480);
      expect(geometry.inset).toBeGreaterThanOrEqual(previous);
      expect(geometry.inset - previous).toBeLessThan(0.01);
      expect(geometry.radius).toBeLessThanOrEqual(Math.min(width, 172) / 2 - geometry.inset);
      previous = geometry.inset;
    }
  });
});
