import { NodeList } from "./components/editor/NodeList";
import { NodeForm } from "./components/editor/NodeForm";
import { SettingsPanel } from "./components/Settings/SettingsPanel";
import "./EditorRoot.css";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import { normalizeThemeValue } from "./components/editor/SettingsPanel.logic";
import { useSettingsStore } from "./store/settingsStore";

export default function EditorRoot() {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const setAppIndexCount = useSettingsStore((s) => s.setAppIndexCount);

  useEffect(() => {
    let unlistenOpen: (() => void) | undefined;
    let unlistenIndex: (() => void) | undefined;
    void listen("open-settings", () => {
      setSettingsOpen(true);
    }).then((u) => {
      unlistenOpen = u;
    });
    void listen<{ count: number }>("app-index-ready", (event) => {
      setAppIndexCount(event.payload.count);
    }).then((u) => {
      unlistenIndex = u;
    });
    return () => {
      unlistenOpen?.();
      unlistenIndex?.();
    };
  }, [setAppIndexCount]);

  useEffect(() => {
    let mounted = true;
    void invoke<string | null>("get_setting", { key: "theme" })
      .then((savedTheme) => {
        if (!mounted) return;
        const theme = normalizeThemeValue(savedTheme);
        document.documentElement.setAttribute("data-theme", theme);
      })
      .catch(() => {
        if (!mounted) return;
        document.documentElement.setAttribute("data-theme", "dark");
      });
    return () => {
      mounted = false;
    };
  }, []);

  return (
    <main className="editor-root">
      <header className="editor-root-header">
        <h1>JARVIS Command Editor</h1>
        <button
          type="button"
          className="editor-gear-btn"
          onClick={() => setSettingsOpen((open) => !open)}
          aria-label={settingsOpen ? "Close settings panel" : "Open settings panel"}
          aria-pressed={settingsOpen}
        >
          ⚙
        </button>
      </header>
      <div className="editor-root-content">
        <NodeList />
        <NodeForm />
      </div>
      {settingsOpen && <SettingsPanel onClose={() => setSettingsOpen(false)} />}
    </main>
  );
}
