import { CommandsTab } from "./components/editor/CommandsTab";
import { EDITOR_SETTINGS_NAV, type EditorSettingsNavId } from "./components/Settings/settingsNav";
import { SettingsPanel } from "./components/Settings/SettingsPanel";
import "./EditorRoot.css";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import {
  applyEditorThemeToDocument,
  normalizeThemePreference,
} from "./components/editor/SettingsPanel.logic";
import { useEditorStore } from "./store/editorStore";
import { useSettingsStore } from "./store/settingsStore";

type ShellSection = "commands" | EditorSettingsNavId;

export default function EditorRoot() {
  const nodes = useEditorStore((s) => s.nodes);
  const setAppIndexCount = useSettingsStore((s) => s.setAppIndexCount);
  const [section, setSection] = useState<ShellSection>("commands");

  useEffect(() => {
    let unlistenOpen: (() => void) | undefined;
    let unlistenIndex: (() => void) | undefined;
    void listen("open-settings", () => {
      setSection("recognition");
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
        const pref = normalizeThemePreference(savedTheme);
        applyEditorThemeToDocument(pref);
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

  return (
    <main className="editor-app">
      <div className="editor-app-shell editor-glass-panel">
        <div className="editor-window-chrome" data-tauri-drag-region aria-hidden="true" />
        <aside className="editor-app-sidebar" aria-label="JARVIS navigation">
          <div className="editor-app-brand">JARVIS</div>

          <div className="editor-app-nav-group">
            <div className="editor-app-nav-label">Library</div>
            <button
              type="button"
              className={`editor-app-nav-btn${section === "commands" ? " is-active" : ""}`}
              onClick={() => setSection("commands")}
            >
              <span className="editor-app-nav-label-inline">Commands</span>
              <span className="editor-app-badge" aria-label={`${nodes.length} commands`}>
                {nodes.length}
              </span>
            </button>
          </div>

          <div className="editor-app-nav-group">
            <div className="editor-app-nav-label">Settings</div>
            {EDITOR_SETTINGS_NAV.map((item) => (
              <button
                key={item.id}
                type="button"
                className={`editor-app-nav-btn${section === item.id ? " is-active" : ""}`}
                onClick={() => setSection(item.id)}
              >
                {item.label}
              </button>
            ))}
          </div>
        </aside>

        <section className="editor-app-main" aria-label="Editor content">
          {section === "commands" ? (
            <CommandsTab />
          ) : (
            <SettingsPanel embedded activeNav={section} />
          )}
        </section>
      </div>
    </main>
  );
}
