import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const appCss = readFileSync(new URL("../App.css", import.meta.url), "utf8");
const editorCss = readFileSync(new URL("../EditorRoot.css", import.meta.url), "utf8");
const tokensCss = readFileSync(new URL("../glassTokens.css", import.meta.url), "utf8");

describe("glass style parity", () => {
  it("uses denser shared level-1 fill for primary panels", () => {
    expect(tokensCss).toMatch(/--glass-1-fill:\s*52%/);
  });

  it("keeps hud root on shared glass tokens", () => {
    expect(appCss).toContain(".hud-root");
    expect(appCss).not.toMatch(/--hud-panel-fill-ratio/);
    expect(appCss).not.toMatch(/--hud-panel-saturate/);
    expect(appCss).not.toMatch(/--hud-panel-blur/);
  });

  it("keeps editor root transparency chain for backdrop-filter", () => {
    expect(editorCss).toMatch(/html[\s\S]*background:\s*transparent;/);
    expect(editorCss).toMatch(/#root[\s\S]*background:\s*transparent;/);
  });

  it("dark theme glass fills stay slightly below full opacity for translucency", () => {
    const darkBlock = tokensCss.match(/\[data-theme="dark"\]\s*\{[\s\S]*?\}/)?.[0] ?? "";
    expect(darkBlock).toMatch(/--glass-0-fill:\s*88%/);
    expect(darkBlock).toMatch(/--glass-1-fill:\s*90%/);
    expect(darkBlock).toMatch(/--glass-2-fill:\s*90%/);
  });

  it("defines complete semantic color tokens for dark theme", () => {
    const darkBlock = tokensCss.match(/\[data-theme="dark"\]\s*\{[\s\S]*?\}/)?.[0] ?? "";
    expect(darkBlock).toMatch(/--color-bg-app:/);
    expect(darkBlock).toMatch(/--color-surface-main:/);
    expect(darkBlock).toMatch(/--color-surface-elevated:/);
    expect(darkBlock).toMatch(/--color-surface-sidebar:/);
    expect(darkBlock).toMatch(/--color-divider:/);
    expect(darkBlock).toMatch(/--color-text-primary:/);
    expect(darkBlock).toMatch(/--color-text-secondary:/);
    expect(darkBlock).toMatch(/--color-accent:/);
  });
});
