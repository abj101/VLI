import { NodeList } from "./components/editor/NodeList";
import { NodeForm } from "./components/editor/NodeForm";
import { SettingsPanel } from "./components/Settings/SettingsPanel";
import "./EditorRoot.css";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import {
  applyEditorThemeToDocument,
  normalizeThemePreference,
} from "./components/editor/SettingsPanel.logic";
import { useSettingsStore } from "./store/settingsStore";

export default function EditorRoot() {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const setAppIndexCount = useSettingsStore((s) => s.setAppIndexCount);
  const gearBtnRef = useRef<HTMLButtonElement>(null);

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

  /** When preference is system, keep `data-theme` in sync with OS color scheme. */
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
    <main className="editor-root">
      <header className="editor-root-header">
        <h1>JARVIS Command Editor</h1>
        <button
          ref={gearBtnRef}
          type="button"
          className="editor-gear-btn"
          onClick={() => setSettingsOpen((open) => !open)}
          aria-label={settingsOpen ? "Close settings panel" : "Open settings panel"}
          aria-expanded={settingsOpen}
          aria-haspopup="dialog"
        >
          <svg width="22" height="22" viewBox="0 0 24 24" aria-hidden="true" focusable={false}>
            <path
              fill="currentColor"
              d="M12 8.4a3.6 3.6 0 1 0 0 7.2 3.6 3.6 0 0 0 0-7.2Zm7.9 3.3-.9-.3a6.5 6.5 0 0 0-.3-.8l.6-.8a.8.8 0 0 0-.1-1l-1-1a.8.8 0 0 0-1-.1l-.8.6a6.5 6.5 0 0 0-.8-.3l-.3-.9a.8.8 0 0 0-.7-.5h-1.4a.8.8 0 0 0-.7.5l-.3.9c-.3.1-.5.2-.8.3l-.8-.6a.8.8 0 0 0-1 .1l-1 1a.8.8 0 0 0-.1 1l.6.8c-.1.2-.2.5-.3.8l-.9.3a.8.8 0 0 0-.5.7v1.4c0 .3.2.6.5.7l.9.3c.1.3.2.5.3.8l-.6.8a.8.8 0 0 0 .1 1l1 1c.3.3.8.3 1 .1l.8-.6c.2.1.5.2.8.3l.3.9c.1.3.4.5.7.5h1.4c.3 0 .6-.2.7-.5l.3-.9c.3-.1.5-.2.8-.3l.8.6c.3.2.8.1 1-.1l1-1a.8.8 0 0 0 .1-1l-.6-.8c.1-.2.2-.5.3-.8l.9-.3a.8.8 0 0 0 .5-.7v-1.4a.8.8 0 0 0-.5-.7Z"
            />
          </svg>
        </button>
      </header>
      <div className="editor-root-content">
        <NodeList />
        <NodeForm />
      </div>
      {settingsOpen && (
        <>
          <div
            className="editor-settings-backdrop"
            aria-hidden
            onClick={() => setSettingsOpen(false)}
          />
          <SettingsPanel
            onClose={() => setSettingsOpen(false)}
            returnFocusRef={gearBtnRef}
          />
        </>
      )}
    </main>
  );
}
