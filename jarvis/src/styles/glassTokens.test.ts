// @ts-nocheck
import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const testDir = dirname(fileURLToPath(import.meta.url));
const appStyles = readFileSync(resolve(testDir, "../App.css"), "utf8");
const editorStyles = readFileSync(resolve(testDir, "../EditorRoot.css"), "utf8");
const sharedGlassStyles = readFileSync(resolve(testDir, "../glassTokens.css"), "utf8");
const hudPanelSource = readFileSync(resolve(testDir, "../components/hud/HudPanel.tsx"), "utf8");

describe("unified glass tokens", () => {
  it("defines shared glass levels in App root", () => {
    expect(sharedGlassStyles).toContain("--glass-0-blur: 18px;");
    expect(sharedGlassStyles).toContain("--glass-1-blur: 24px;");
    expect(sharedGlassStyles).toContain("--glass-2-blur: 16px;");
    expect(sharedGlassStyles).toContain("--glass-specular-sheen:");
  });

  it("provides root-level fallback surface tokens", () => {
    expect(sharedGlassStyles).toMatch(
      /:root\s*\{[\s\S]*--glass-surface:\s*oklch\(0\.16 0\.008 248\);[\s\S]*--glass-border-top:\s*oklch\(1 0 0 \/ 0\.2\);[\s\S]*--glass-border-side:\s*oklch\(1 0 0 \/ 0\.07\);[\s\S]*--glass-text:\s*oklch\(0\.96 0 0\);/m,
    );
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

  it("keeps embedded settings on a flat layer", () => {
    expect(editorStyles).toMatch(
      /\.editor-settings-embedded\s*\{[\s\S]*background:\s*transparent;[\s\S]*backdrop-filter:\s*none;[\s\S]*-webkit-backdrop-filter:\s*none;[\s\S]*box-shadow:\s*none;[\s\S]*border:\s*none;/m,
    );
  });

  it("mounts HUD root with shared level-1 glass class", () => {
    expect(hudPanelSource).toContain('className="hud-root glass-panel-1"');
  });

  it("tunes HUD glass to a lighter, more transparent material", () => {
    expect(appStyles).toContain("--hud-panel-fill-ratio: 38%;");
    expect(appStyles).toContain("--glass-1-fill: var(--hud-panel-fill-ratio);");
  });
});
