import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import {
  applyEditorThemeToDocument,
  normalizeThemePreference,
} from "./components/editor/SettingsPanel.logic";
import { HudPanel } from "./components/hud/HudPanel";
import { subscribeHudIpc } from "./store/hudIpc";
import "./App.css";

export default function App() {
  useEffect(() => {
    let mounted = true;
    void invoke<string | null>("get_setting", { key: "theme" })
      .then((savedTheme) => {
        if (!mounted) return;
        applyEditorThemeToDocument(normalizeThemePreference(savedTheme));
      })
      .catch(() => {
        if (!mounted) return;
        applyEditorThemeToDocument("system");
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
    let unlisten: (() => void) | undefined;
    void listen<{ preference?: string }>("theme-preference-changed", (e) => {
      const raw = e.payload.preference;
      applyEditorThemeToDocument(normalizeThemePreference(raw ?? null));
    }).then((u) => {
      unlisten = u;
    });
    return () => {
      unlisten?.();
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

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      void invoke("hud_dismiss").catch(() => {});
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return <HudPanel />;
}
