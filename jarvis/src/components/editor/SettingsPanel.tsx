import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import {
  normalizeThemeValue,
  parseThresholdSettingValue,
  validateHotkeyInput,
  type EditorTheme,
} from "./SettingsPanel.logic";

const HOTKEY_KEY = "hotkey";
const THEME_KEY = "theme";
const DEFAULT_THRESHOLD_KEY = "default_fuzzy_threshold_pct";

function applyTheme(theme: EditorTheme) {
  document.documentElement.setAttribute("data-theme", theme);
}

type SettingsPanelProps = {
  onClose: () => void;
};

export function SettingsPanel({ onClose }: SettingsPanelProps) {
  const [loading, setLoading] = useState(true);
  const [hotkey, setHotkey] = useState("ctrl+shift+j");
  const [threshold, setThreshold] = useState(0.8);
  const [theme, setTheme] = useState<EditorTheme>("dark");
  const [hotkeyError, setHotkeyError] = useState<string | null>(null);
  const [toastText, setToastText] = useState<string | null>(null);
  const [savingHotkey, setSavingHotkey] = useState(false);
  const [anthropicKeyConfigured, setAnthropicKeyConfigured] = useState<boolean | null>(null);

  useEffect(() => {
    let mounted = true;
    const load = async () => {
      try {
        const [savedHotkey, savedThreshold, savedTheme, aiKeyOk] = await Promise.all([
          invoke<string | null>("get_setting", { key: HOTKEY_KEY }),
          invoke<string | null>("get_setting", { key: DEFAULT_THRESHOLD_KEY }),
          invoke<string | null>("get_setting", { key: THEME_KEY }),
          invoke<boolean>("anthropic_api_key_configured"),
        ]);
        if (!mounted) return;
        if (savedHotkey && savedHotkey.trim().length > 0) {
          setHotkey(savedHotkey.trim());
        }
        const parsedThreshold = parseThresholdSettingValue(savedThreshold);
        if (parsedThreshold !== null) {
          setThreshold(parsedThreshold);
        }
        const normalizedTheme = normalizeThemeValue(savedTheme);
        setTheme(normalizedTheme);
        applyTheme(normalizedTheme);
        setAnthropicKeyConfigured(aiKeyOk);
      } catch (err) {
        if (!mounted) return;
        setToastText(`Failed to load settings: ${String(err)}`);
        setAnthropicKeyConfigured(false);
      } finally {
        if (mounted) {
          setLoading(false);
        }
      }
    };
    void load();
    return () => {
      mounted = false;
    };
  }, []);

  const saveThreshold = async (nextThreshold: number) => {
    const pct = Math.round(nextThreshold * 100);
    try {
      await invoke("set_setting", {
        key: DEFAULT_THRESHOLD_KEY,
        value: String(pct),
      });
    } catch (err) {
      setToastText(`Failed to save threshold: ${String(err)}`);
    }
  };

  const saveTheme = async (nextTheme: EditorTheme) => {
    applyTheme(nextTheme);
    setTheme(nextTheme);
    try {
      await invoke("set_setting", { key: THEME_KEY, value: nextTheme });
    } catch (err) {
      setToastText(`Failed to save theme: ${String(err)}`);
    }
  };

  const refreshAnthropicStatus = async () => {
    try {
      const ok = await invoke<boolean>("anthropic_api_key_configured");
      setAnthropicKeyConfigured(ok);
    } catch {
      setAnthropicKeyConfigured(false);
    }
  };

  const saveHotkey = async () => {
    const maybeError = validateHotkeyInput(hotkey);
    if (maybeError) {
      setHotkeyError(maybeError);
      return;
    }
    setHotkeyError(null);
    setSavingHotkey(true);
    try {
      const savedHotkey = await invoke<string>("set_hotkey", { hotkey });
      setHotkey(savedHotkey);
      setToastText("Hotkey updated");
    } catch (err) {
      setHotkeyError(String(err));
    } finally {
      setSavingHotkey(false);
    }
  };

  return (
    <aside className="editor-settings-panel" role="dialog" aria-label="Settings">
      <header className="editor-settings-header">
        <h2>Settings</h2>
        <button type="button" onClick={onClose} aria-label="Close settings">
          Close
        </button>
      </header>

      {loading ? (
        <p className="editor-settings-loading">Loading settings...</p>
      ) : (
        <div className="editor-settings-content">
          <section className="editor-settings-section">
            <h3>Hotkey</h3>
            <label>
              Global shortcut
              <div className="editor-settings-inline">
                <input
                  value={hotkey}
                  onChange={(e) => setHotkey(e.target.value)}
                  placeholder="ctrl+shift+j"
                />
                <button type="button" onClick={saveHotkey} disabled={savingHotkey}>
                  {savingHotkey ? "Saving..." : "Save"}
                </button>
              </div>
            </label>
            {hotkeyError && <p className="editor-field-error">{hotkeyError}</p>}
          </section>

          <section className="editor-settings-section">
            <h3>Default fuzzy threshold</h3>
            <label>
              {threshold.toFixed(2)}
              <input
                type="range"
                min={0.5}
                max={1}
                step={0.01}
                value={threshold}
                onChange={(e) => {
                  const nextThreshold = Number(e.target.value);
                  setThreshold(nextThreshold);
                  void saveThreshold(nextThreshold);
                }}
              />
            </label>
          </section>

          <section className="editor-settings-section">
            <h3>Theme</h3>
            <label>
              Theme
              <select
                value={theme}
                onChange={(e) => {
                  void saveTheme(normalizeThemeValue(e.target.value));
                }}
              >
                <option value="dark">Dark</option>
                <option value="light">Light</option>
              </select>
            </label>
          </section>

          <section className="editor-settings-section">
            <h3>AI mode</h3>
            <p className="editor-settings-help">
              Commands with a sub-prompt can run an AI preview after their action chain (model{" "}
              <code className="editor-settings-code">claude-haiku-4-5</code>). Set the key in your
              environment before starting the app.
            </p>
            <p className="editor-settings-help" role="status">
              {anthropicKeyConfigured === null
                ? "Checking API key…"
                : anthropicKeyConfigured
                  ? "Anthropic API key: configured (not shown here)."
                  : "Anthropic API key: not set — set ANTHROPIC_API_KEY in the environment."}
            </p>
            <button type="button" className="editor-settings-secondary-btn" onClick={() => void refreshAnthropicStatus()}>
              Refresh status
            </button>
          </section>
        </div>
      )}

      {toastText && (
        <div className="editor-inline-toast" role="status">
          {toastText}
        </div>
      )}
    </aside>
  );
}
