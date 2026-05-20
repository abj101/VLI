import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { invoke } from "@tauri-apps/api/core";
import {
  applyEditorThemeToDocument,
  hotkeyChordMatchesKeyboardEvent,
  normalizeThemePreference,
} from "./components/editor/SettingsPanel.logic";
import { HudPanel } from "./components/hud/HudPanel";
import { subscribeHudIpc } from "./store/hudIpc";
import "./App.css";

export default function App() {
  const [dismissHotkeyChord, setDismissHotkeyChord] = useState("escape");
  const dismissChordRef = useRef("escape");
  dismissChordRef.current = dismissHotkeyChord;

  /** WebView2 on Win: alpha≠0 in host `backgroundColor` → opaque backing → ghost when DOM fades. */
  useEffect(() => {
    void getCurrentWebview()
      .setBackgroundColor([0, 0, 0, 0])
      .catch(() => {});
  }, []);

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
    let mounted = true;
    void invoke<string | null>("get_setting", { key: "dismiss_hotkey" })
      .then((raw) => {
        if (!mounted || !raw?.trim()) return;
        setDismissHotkeyChord(raw.trim());
      })
      .catch(() => {});
    return () => {
      mounted = false;
    };
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<{ hotkey?: string }>("dismiss-hotkey-changed", (e) => {
      const raw = e.payload.hotkey;
      if (raw?.trim()) setDismissHotkeyChord(raw.trim());
    }).then((u) => {
      unlisten = u;
    });
    return () => {
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!hotkeyChordMatchesKeyboardEvent(dismissChordRef.current, e)) return;
      e.preventDefault();
      void invoke("hud_dismiss").catch(() => {});
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return <HudPanel dismissHotkeyChord={dismissHotkeyChord} />;
}
