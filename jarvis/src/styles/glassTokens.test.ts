// @ts-nocheck
import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const testDir = dirname(fileURLToPath(import.meta.url));
const appStyles = readFileSync(resolve(testDir, "../App.css"), "utf8");
const editorStyles = readFileSync(resolve(testDir, "../EditorRoot.css"), "utf8");

describe("unified glass tokens", () => {
  it("defines shared glass levels in App root", () => {
    expect(appStyles).toContain("--glass-0-blur: 18px;");
    expect(appStyles).toContain("--glass-1-blur: 24px;");
    expect(appStyles).toContain("--glass-2-blur: 16px;");
    expect(appStyles).toContain("--glass-specular-sheen:");
  });

  it("uses shared data-theme glass surface tokens", () => {
    expect(appStyles).toContain('[data-theme="dark"]');
    expect(appStyles).toContain("--glass-surface: oklch(0.16 0.008 248);");
    expect(appStyles).toContain('[data-theme="light"]');
    expect(appStyles).toContain("--glass-surface: oklch(0.98 0.002 248);");
  });

  it("removes legacy component glass token names", () => {
    expect(appStyles).not.toMatch(/--hud-glass-/);
    expect(editorStyles).not.toMatch(/--editor-glass-/);
  });

  it("caps explicit blur values at 32px in editor overlays", () => {
    const blurMatches = Array.from(editorStyles.matchAll(/blur\((\d+)px\)/g));
    expect(blurMatches.length).toBeGreaterThan(0);
    for (const match of blurMatches) {
      const blur = Number(match[1]);
      expect(blur).toBeLessThanOrEqual(32);
    }
  });
});
