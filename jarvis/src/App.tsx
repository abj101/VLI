import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { invoke } from "@tauri-apps/api/core";
import {
  applyEditorThemeToDocument,
  applyHudTransparencyToDocument,
  HUD_TRANSPARENCY_DEFAULT,
  normalizeThemePreference,
  parseHudTransparencySettingValue,
} from "./components/editor/SettingsPanel.logic";
import { HudPanel } from "./components/hud/HudPanel";
import { subscribeHudIpc } from "./store/hudIpc";
import "./App.css";

export default function App() {
  /** WebView2 on Win: alpha≠0 in host `backgroundColor` → opaque backing → ghost when DOM fades. */
  useEffect(() => {
    void getCurrentWebview()
      .setBackgroundColor([0, 0, 0, 0])
      .catch(() => {});
  }, []);

  useEffect(() => {
    let mounted = true;
    void Promise.all([
      invoke<string | null>("get_setting", { key: "theme" }),
      invoke<string | null>("get_setting", { key: "hud_transparency" }),
    ])
      .then(([savedTheme, savedTransparency]) => {
        if (!mounted) return;
        const pref = normalizeThemePreference(savedTheme);
        applyEditorThemeToDocument(pref);
        applyHudTransparencyToDocument(parseHudTransparencySettingValue(savedTransparency), pref);
      })
      .catch(() => {
        if (!mounted) return;
        applyEditorThemeToDocument("system");
        applyHudTransparencyToDocument(HUD_TRANSPARENCY_DEFAULT, "system");
      });
    return () => {
      mounted = false;
    };
  }, []);

  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: light)");
    const sync = () => {
      if (document.documentElement.getAttribute("data-theme-preference") !== "system") {
        return;
      }
      document.documentElement.setAttribute("data-theme", mq.matches ? "light" : "dark");
    };
    sync();
    mq.addEventListener("change", sync);
    return () => mq.removeEventListener("change", sync);
  }, []);

  useEffect(() => {
    let unlistenTheme: (() => void) | undefined;
    let unlistenTransparency: (() => void) | undefined;
    void listen<{ preference?: string }>("theme-preference-changed", (e) => {
      const pref = normalizeThemePreference(e.payload.preference ?? null);
      applyEditorThemeToDocument(pref);
      void invoke<string | null>("get_setting", { key: "hud_transparency" }).then((raw) => {
        applyHudTransparencyToDocument(parseHudTransparencySettingValue(raw), pref);
      });
    }).then((u) => {
      unlistenTheme = u;
    });
    void listen<{ transparency?: string }>("hud-transparency-changed", (e) => {
      applyHudTransparencyToDocument(parseHudTransparencySettingValue(e.payload.transparency ?? null));
    }).then((u) => {
      unlistenTransparency = u;
    });
    return () => {
      unlistenTheme?.();
      unlistenTransparency?.();
    };
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void subscribeHudIpc().then((u) => {
      unlisten = u;
    });
    return () => {
      unlisten?.();
    };
  }, []);

  return <HudPanel />;
}
