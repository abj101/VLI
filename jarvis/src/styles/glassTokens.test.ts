// @ts-nocheck
import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const testDir = dirname(fileURLToPath(import.meta.url));
const appStyles = readFileSync(resolve(testDir, "../App.css"), "utf8");
const editorStyles = readFileSync(resolve(testDir, "../EditorRoot.css"), "utf8");
const sharedGlassStyles = readFileSync(resolve(testDir, "../glassTokens.css"), "utf8");

describe("unified glass tokens", () => {
  it("defines shared glass levels in App root", () => {
    expect(sharedGlassStyles).toContain("--glass-0-blur: 18px;");
    expect(sharedGlassStyles).toContain("--glass-1-blur: 24px;");
    expect(sharedGlassStyles).toContain("--glass-2-blur: 16px;");
    expect(sharedGlassStyles).toContain("--glass-specular-sheen:");
  });

  it("uses shared data-theme glass surface tokens", () => {
    expect(sharedGlassStyles).toContain('[data-theme="dark"]');
    expect(sharedGlassStyles).toContain("--glass-surface: oklch(0.16 0.008 248);");
    expect(sharedGlassStyles).toContain('[data-theme="light"]');
    expect(sharedGlassStyles).toContain("--glass-surface: oklch(0.98 0.002 248);");
  });

  it("imports shared glass tokens in HUD and editor styles", () => {
    expect(appStyles).toContain('@import "./glassTokens.css";');
    expect(editorStyles).toContain('@import "./glassTokens.css";');
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
