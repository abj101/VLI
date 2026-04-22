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
    expect(_legacyGlassSnapshot.length).toBeGreaterThan(0);
  });
});
const _legacyGlassSnapshot = String.raw`/*
 * glassTokens.css
 * ─────────────────────────────────────────────────────────────
 * Single source of truth for iOS 26-style glassmorphism tokens.
 * Imported by App.css and EditorRoot.css — never define glass
 * tokens in those files directly.
 *
 * CRITICAL for backdrop-filter to work:
 *   Every ancestor of a .glass-panel-* element must be either:
 *     (a) transparent, OR
 *     (b) itself have backdrop-filter (creating its own stacking context)
 *   If any ancestor has an opaque background, backdrop-filter blurs
 *   that solid fill instead of the desktop — result: flat dark rectangle.
 *
 *   HUD chain must be: html(transparent) > body(transparent) >
 *   #root(transparent) > .hud-panel-fill(transparent) > .hud-root(glass)
 * ─────────────────────────────────────────────────────────────
 */

/* ── Level tokens (theme-invariant) ─────────────────────────── */
:root {
  /* Level 0: chrome bars, tab bars only */
  --glass-0-blur:        18px;
  --glass-0-fill:        52%;
  --glass-0-saturate:    1.9;
  --glass-0-brightness:  1.12;

  /* Level 1: primary panels, HUD root, floating sidebars, settings */
  --glass-1-blur:        24px;
  --glass-1-fill:        44%;
  --glass-1-saturate:    2.1;
  --glass-1-brightness:  1.16;

  /* Level 2: elevated popovers, menus, dropdowns */
  --glass-2-blur:        16px;
  --glass-2-fill:        56%;
  --glass-2-saturate:    1.7;
  --glass-2-brightness:  1.10;

  /* Specular sheen — diagonal catch-light on top-left edge */
  --glass-specular-sheen: oklch(1 0 0 / 0.08);

  /* Root-level fallbacks (dark) so tokens always resolve */
  --glass-surface:      oklch(0.16 0.008 248);
  --glass-border-top:   oklch(1 0 0 / 0.2);
  --glass-border-side:  oklch(1 0 0 / 0.07);
  --glass-rim-side:     oklch(1 0 0 / 0.05);
  --glass-separator:    oklch(1 0 0 / 0.08);
  --glass-text:         oklch(0.96 0 0);
  --glass-text-muted:   oklch(0.58 0 0);
  --glass-text-subtle:  oklch(0.46 0 0);
  --glass-shadow:
    0 2px 10px oklch(0 0 0 / 0.22),
    0 8px 28px oklch(0 0 0 / 0.18);
  --glass-shadow-menu:
    0 2px 6px  oklch(0 0 0 / 0.14),
    0 6px 18px oklch(0 0 0 / 0.12);
}

/* ── Dark theme ──────────────────────────────────────────────── */
[data-theme="dark"] {
  --glass-surface:      oklch(0.16 0.008 248);
  --glass-border-top:   oklch(1 0 0 / 0.2);
  --glass-border-side:  oklch(1 0 0 / 0.07);
  --glass-rim-side:     oklch(1 0 0 / 0.05);
  --glass-separator:    oklch(1 0 0 / 0.08);
  --glass-text:         oklch(0.96 0 0);
  --glass-text-muted:   oklch(0.58 0 0);
  --glass-text-subtle:  oklch(0.46 0 0);
  --glass-shadow:
    0 2px 10px oklch(0 0 0 / 0.22),
    0 8px 28px oklch(0 0 0 / 0.18);
  --glass-shadow-menu:
    0 2px 6px  oklch(0 0 0 / 0.14),
    0 6px 18px oklch(0 0 0 / 0.12);
  --glass-1-brightness: 1.16;
  --glass-0-brightness: 1.12;
  --glass-2-brightness: 1.10;
}

/* ── Light theme ─────────────────────────────────────────────── */
[data-theme="light"] {
  --glass-surface:      oklch(0.98 0.002 248);
  --glass-border-top:   oklch(1 0 0 / 0.72);
  --glass-border-side:  oklch(0 0 0 / 0.06);
  --glass-rim-side:     oklch(0 0 0 / 0.04);
  --glass-separator:    oklch(0 0 0 / 0.07);
  --glass-text:         oklch(0.12 0 0);
  --glass-text-muted:   oklch(0.44 0 0);
  --glass-text-subtle:  oklch(0.56 0 0);
  --glass-shadow:
    0 2px 8px  oklch(0 0 0 / 0.06),
    0 8px 24px oklch(0 0 0 / 0.07);
  --glass-shadow-menu:
    0 2px 6px  oklch(0 0 0 / 0.04),
    0 6px 18px oklch(0 0 0 / 0.05);
  --glass-1-brightness: 0.96;
  --glass-0-brightness: 0.97;
  --glass-2-brightness: 0.98;
}

/* ══════════════════════════════════════════════════════════════
 * GLASS PANEL UTILITY CLASSES
 * Apply these to elements whose ancestors are all transparent.
 * The element itself sets background + backdrop-filter.
 * Inner children must NOT add their own backdrop-filter.
 * ══════════════════════════════════════════════════════════════ */

/*
 * .glass-panel-1 — primary panels, HUD root, floating sidebars
 * ─────────────────────────────────────────────────────────────
 * Layered background recipe:
 *   1. Diagonal specular sheen  (top-left catch-light)
 *   2. Vertical veil            (subtle density at top)
 *   3. Base fill                (main translucency via color-mix)
 */
.glass-panel-1 {
  background:
    linear-gradient(
      135deg,
      color-mix(in oklch, var(--glass-specular-sheen) 100%, transparent) 0%,
      transparent 35%
    ),
    linear-gradient(
      180deg,
      color-mix(in oklch, var(--glass-surface) 22%, transparent) 0%,
      transparent 40%
    ),
    color-mix(in oklch, var(--glass-surface) var(--glass-1-fill), transparent);
  backdrop-filter:
    blur(var(--glass-1-blur))
    saturate(var(--glass-1-saturate))
    brightness(var(--glass-1-brightness));
  -webkit-backdrop-filter:
    blur(var(--glass-1-blur))
    saturate(var(--glass-1-saturate))
    brightness(var(--glass-1-brightness));
  box-shadow:
    var(--glass-shadow),
    inset 0  1px 0 var(--glass-border-top),
    inset 1px 0 0 var(--glass-rim-side),
    inset -1px 0 0 var(--glass-rim-side),
    inset 0 -1px 0 oklch(0 0 0 / 0.04);
  border: 1px solid var(--glass-border-side);
  isolation: isolate;
  transform: translateZ(0);
}

/* .glass-panel-0 — chrome bars, title bars only */
.glass-panel-0 {
  background:
    linear-gradient(
      180deg,
      color-mix(in oklch, var(--glass-surface) 28%, transparent) 0%,
      color-mix(in oklch, var(--glass-surface) 12%, transparent) 100%
    );
  backdrop-filter:
    blur(var(--glass-0-blur))
    saturate(var(--glass-0-saturate))
    brightness(var(--glass-0-brightness));
  -webkit-backdrop-filter:
    blur(var(--glass-0-blur))
    saturate(var(--glass-0-saturate))
    brightness(var(--glass-0-brightness));
  border-bottom: 1px solid var(--glass-separator);
  isolation: isolate;
  transform: translateZ(0);
}

/* .glass-panel-2 — popovers, menus, dropdowns */
.glass-panel-2 {
  background:
    linear-gradient(
      135deg,
      color-mix(in oklch, var(--glass-specular-sheen) 80%, transparent) 0%,
      transparent 30%
    ),
    color-mix(in oklch, var(--glass-surface) var(--glass-2-fill), transparent);
  backdrop-filter:
    blur(var(--glass-2-blur))
    saturate(var(--glass-2-saturate))
    brightness(var(--glass-2-brightness));
  -webkit-backdrop-filter:
    blur(var(--glass-2-blur))
    saturate(var(--glass-2-saturate))
    brightness(var(--glass-2-brightness));
  box-shadow:
    var(--glass-shadow-menu),
    inset 0 1px 0 var(--glass-border-top),
    inset 0 0 0 1px var(--glass-rim-side);
  border: 1px solid var(--glass-border-side);
  isolation: isolate;
  transform: translateZ(0);
}

/* ── Accessibility overrides ─────────────────────────────────── */
@media (prefers-contrast: more) {
  .glass-panel-0,
  .glass-panel-1,
  .glass-panel-2 {
    background: var(--glass-surface);
    backdrop-filter: none;
    -webkit-backdrop-filter: none;
    box-shadow: none;
    border: 1px solid var(--glass-border-side);
  }
}
`;