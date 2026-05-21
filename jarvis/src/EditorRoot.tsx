import { CommandsTab } from "./components/editor/CommandsTab";
import { EDITOR_SETTINGS_NAV, type EditorSettingsNavId } from "./components/Settings/settingsNav";
import { SettingsPanel } from "./components/Settings/SettingsPanel";
import "./EditorRoot.css";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useState } from "react";
import {
  applyEditorThemeToDocument,
  normalizeThemePreference,
} from "./components/editor/SettingsPanel.logic";
import { useSettingsStore } from "./store/settingsStore";

type ShellSection = "commands" | EditorSettingsNavId;

function CaptionMinimizeIcon() {
  return (
    <svg className="editor-caption-icon-svg" viewBox="0 0 10 10" width="10" height="10" aria-hidden>
      <rect x="1" y="5" width="8" height="1" fill="currentColor" />
    </svg>
  );
}

function CaptionMaximizeIcon() {
  return (
    <svg className="editor-caption-icon-svg" viewBox="0 0 10 10" width="10" height="10" aria-hidden>
      <rect x="1.5" y="1.5" width="7" height="7" fill="none" stroke="currentColor" strokeWidth="1" />
    </svg>
  );
}

function CaptionRestoreIcon() {
  return (
    <svg className="editor-caption-icon-svg" viewBox="0 0 12 12" width="12" height="12" aria-hidden>
      <rect x="3.5" y="1.5" width="7" height="7" fill="none" stroke="currentColor" strokeWidth="1" />
      <rect x="1.5" y="3.5" width="7" height="7" fill="none" stroke="currentColor" strokeWidth="1" />
    </svg>
  );
}

function CaptionCloseIcon() {
  return (
    <svg className="editor-caption-icon-svg" viewBox="0 0 10 10" width="10" height="10" aria-hidden>
      <path
        d="M1 1l8 8M9 1L1 9"
        fill="none"
        stroke="currentColor"
        strokeWidth="1"
        strokeLinecap="square"
      />
    </svg>
  );
}

export default function EditorRoot() {
  const setAppIndexStatus = useSettingsStore((s) => s.setAppIndexStatus);
  const [section, setSection] = useState<ShellSection>("commands");
  const [isMaximized, setIsMaximized] = useState(false);

  const syncMaximized = useCallback(() => {
    const w = getCurrentWindow();
    void w.isMaximized().then(setIsMaximized);
  }, []);

  /** WebView2: non-zero host alpha → square opaque backing outside CSS clip. */
  useEffect(() => {
    void getCurrentWebview()
      .setBackgroundColor([0, 0, 0, 0])
      .catch(() => {});
  }, []);

  useEffect(() => {
    void syncMaximized();
    const onResize = () => {
      void syncMaximized();
    };
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, [syncMaximized]);

  useEffect(() => {
    let unlistenOpen: (() => void) | undefined;
    let unlistenIndex: (() => void) | undefined;
    void listen("open-settings", () => {
      setSection("recognition");
    }).then((u) => {
      unlistenOpen = u;
    });
    void listen<{ count: number; scanning?: boolean }>("app-index-ready", (event) => {
      setAppIndexStatus({
        count: event.payload.count,
        scanning: event.payload.scanning ?? false,
      });
    }).then((u) => {
      unlistenIndex = u;
    });
    // Also pull the current status on mount in case the scan already completed
    // before the frontend subscribed to the event (opening settings late would
    // otherwise keep the count stuck at "…").
    void invoke<{ count: number; scanning: boolean }>("get_app_index_status")
      .then((status) => {
        setAppIndexStatus(status);
      })
      .catch(() => {
        /* settings panel will stay on "…" until the next event */
      });
    return () => {
      unlistenOpen?.();
      unlistenIndex?.();
    };
  }, [setAppIndexStatus]);

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

  const onMinimize = () => {
    void getCurrentWindow().minimize();
  };

  const onToggleMaximize = () => {
    const w = getCurrentWindow();
    void w.toggleMaximize().then(() => syncMaximized());
  };

  const onClose = () => {
    void getCurrentWindow().close();
  };

  return (
    <main className="editor-app">
      <div className="editor-app-shell editor-glass-panel">
        <header className="editor-window-chrome" aria-label="Window">
          <div className="editor-window-chrome-title" data-tauri-drag-region>
            <span className="editor-window-title">VLI</span>
          </div>
          <div className="editor-window-chrome-drag" data-tauri-drag-region />
          <div className="editor-window-chrome-controls" role="group" aria-label="Window controls">
            <button
              type="button"
              className="editor-caption-btn"
              aria-label="Minimize"
              onClick={onMinimize}
            >
              <CaptionMinimizeIcon />
            </button>
            <button
              type="button"
              className="editor-caption-btn"
              aria-label={isMaximized ? "Restore" : "Maximize"}
              onClick={onToggleMaximize}
            >
              {isMaximized ? <CaptionRestoreIcon /> : <CaptionMaximizeIcon />}
            </button>
            <button
              type="button"
              className="editor-caption-btn editor-caption-btn--close"
              aria-label="Close"
              onClick={onClose}
            >
              <CaptionCloseIcon />
            </button>
          </div>
        </header>
        <aside className="editor-app-sidebar" aria-label="VLI navigation">
          <div className="editor-app-nav-group">
            <div className="editor-app-nav-label">Library</div>
            <button
              type="button"
              className={`editor-app-nav-btn${section === "commands" ? " is-active" : ""}`}
              onClick={() => setSection("commands")}
            >
              Commands
            </button>
          </div>

          <div className="editor-app-nav-group">
            <div className="editor-app-nav-label">Settings</div>
            {EDITOR_SETTINGS_NAV.map((item) => (
              <button
                key={item.id}
                id={`editor-settings-nav-${item.id}`}
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
