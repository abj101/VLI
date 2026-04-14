import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import {
  normalizeThemeValue,
  parseThresholdSettingValue,
  validateHotkeyInput,
  type EditorTheme,
} from "../editor/SettingsPanel.logic";
import { useSettingsStore } from "../../store/settingsStore";

const HOTKEY_KEY = "hotkey";
const THEME_KEY = "theme";
const DEFAULT_THRESHOLD_KEY = "default_fuzzy_threshold_pct";

type AppSettingsPayload = {
  anthropicKeyStored: boolean;
  porcupineKeyStored: boolean;
  wakeEngine: string;
  owwThreshold: number;
  globalAiMode: boolean;
};

function applyTheme(theme: EditorTheme) {
  document.documentElement.setAttribute("data-theme", theme);
}

type SettingsPanelProps = {
  onClose: () => void;
};

export function SettingsPanel({ onClose }: SettingsPanelProps) {
  const appIndexCount = useSettingsStore((s) => s.appIndexCount);

  const [loading, setLoading] = useState(true);
  const [hotkey, setHotkey] = useState("ctrl+shift+j");
  const [threshold, setThreshold] = useState(0.8);
  const [theme, setTheme] = useState<EditorTheme>("dark");
  const [hotkeyError, setHotkeyError] = useState<string | null>(null);
  const [toastText, setToastText] = useState<string | null>(null);
  const [savingHotkey, setSavingHotkey] = useState(false);
  const [anthropicKeyConfigured, setAnthropicKeyConfigured] = useState<boolean | null>(null);

  const [wakeEngine, setWakeEngine] = useState("hotkey");
  const [owwThreshold, setOwwThreshold] = useState(0.5);
  const [globalAiMode, setGlobalAiMode] = useState(false);
  const [anthropicKeyStored, setAnthropicKeyStored] = useState(false);
  const [porcupineKeyStored, setPorcupineKeyStored] = useState(false);

  const [anthropicInput, setAnthropicInput] = useState("");
  const [porcupineInput, setPorcupineInput] = useState("");
  const [savingAnthropic, setSavingAnthropic] = useState(false);
  const [savingPorcupine, setSavingPorcupine] = useState(false);

  const refreshFromBackend = async () => {
    const [savedHotkey, savedThreshold, savedTheme, aiKeyOk, app] = await Promise.all([
      invoke<string | null>("get_setting", { key: HOTKEY_KEY }),
      invoke<string | null>("get_setting", { key: DEFAULT_THRESHOLD_KEY }),
      invoke<string | null>("get_setting", { key: THEME_KEY }),
      invoke<boolean>("anthropic_api_key_configured"),
      invoke<AppSettingsPayload>("get_settings"),
    ]);
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
    setWakeEngine(app.wakeEngine);
    setOwwThreshold(app.owwThreshold);
    setGlobalAiMode(app.globalAiMode);
    setAnthropicKeyStored(app.anthropicKeyStored);
    setPorcupineKeyStored(app.porcupineKeyStored);
  };

  useEffect(() => {
    let mounted = true;
    const load = async () => {
      try {
        await refreshFromBackend();
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

  const persistWakeEngine = async (next: string) => {
    setWakeEngine(next);
    try {
      const s = await invoke<AppSettingsPayload>("update_settings", {
        patch: { wakeEngine: next },
      });
      setOwwThreshold(s.owwThreshold);
      setGlobalAiMode(s.globalAiMode);
    } catch (err) {
      setToastText(`Failed to save wake engine: ${String(err)}`);
    }
  };

  const persistOwwThreshold = async (next: number) => {
    setOwwThreshold(next);
    try {
      await invoke<AppSettingsPayload>("update_settings", {
        patch: { owwThreshold: next },
      });
    } catch (err) {
      setToastText(`Failed to save OWW threshold: ${String(err)}`);
    }
  };

  const persistGlobalAiMode = async (on: boolean) => {
    setGlobalAiMode(on);
    try {
      await invoke<AppSettingsPayload>("update_settings", {
        patch: { globalAiMode: on },
      });
    } catch (err) {
      setToastText(`Failed to save AI mode toggle: ${String(err)}`);
    }
  };

  const saveAnthropicKey = async () => {
    if (!anthropicInput.trim()) {
      setToastText("Enter an API key before saving.");
      return;
    }
    setSavingAnthropic(true);
    try {
      await invoke("save_api_key", { service: "anthropic", key: anthropicInput });
      setAnthropicInput("");
      await refreshFromBackend();
      await refreshAnthropicStatus();
      setToastText("Anthropic key saved to OS keychain");
    } catch (err) {
      setToastText(`Failed to save Anthropic key: ${String(err)}`);
    } finally {
      setSavingAnthropic(false);
    }
  };

  const clearAnthropicKey = async () => {
    setSavingAnthropic(true);
    try {
      await invoke("delete_api_key", { service: "anthropic" });
      await refreshFromBackend();
      await refreshAnthropicStatus();
      setToastText("Anthropic key cleared");
    } catch (err) {
      setToastText(`Failed to clear Anthropic key: ${String(err)}`);
    } finally {
      setSavingAnthropic(false);
    }
  };

  const savePorcupineKey = async () => {
    if (!porcupineInput.trim()) {
      setToastText("Enter an access key before saving.");
      return;
    }
    setSavingPorcupine(true);
    try {
      await invoke("save_api_key", { service: "porcupine", key: porcupineInput });
      setPorcupineInput("");
      await refreshFromBackend();
      setToastText("Porcupine access key saved to OS keychain");
    } catch (err) {
      setToastText(`Failed to save Porcupine key: ${String(err)}`);
    } finally {
      setSavingPorcupine(false);
    }
  };

  const clearPorcupineKey = async () => {
    setSavingPorcupine(true);
    try {
      await invoke("delete_api_key", { service: "porcupine" });
      await refreshFromBackend();
      setToastText("Porcupine key cleared");
    } catch (err) {
      setToastText(`Failed to clear Porcupine key: ${String(err)}`);
    } finally {
      setSavingPorcupine(false);
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
            <h3>Wake engine</h3>
            <label>
              Engine
              <select
                value={wakeEngine}
                onChange={(e) => void persistWakeEngine(e.target.value)}
              >
                <option value="hotkey">Hotkey only</option>
                <option value="porcupine">Porcupine</option>
                <option value="oww">OpenWakeWord</option>
              </select>
            </label>
            <p className="editor-settings-help">
              Hotkey-only matches Phase 1–3 behavior. Porcupine and OWW require models and keys as
              documented in the README.
            </p>
          </section>

          <section className="editor-settings-section">
            <h3>OWW threshold</h3>
            <label>
              {owwThreshold.toFixed(2)}
              <input
                type="range"
                min={0.01}
                max={1}
                step={0.01}
                value={owwThreshold}
                onChange={(e) => {
                  const next = Number(e.target.value);
                  void persistOwwThreshold(next);
                }}
              />
            </label>
            <p className="editor-settings-help">Used when the wake engine is OpenWakeWord.</p>
          </section>

          <section className="editor-settings-section">
            <h3>Global AI mode</h3>
            <label className="editor-settings-inline">
              <input
                type="checkbox"
                checked={globalAiMode}
                onChange={(e) => void persistGlobalAiMode(e.target.checked)}
              />
              <span>Prefer AI-assisted command flow when available</span>
            </label>
          </section>

          <section className="editor-settings-section">
            <h3>Anthropic API key</h3>
            <p className="editor-settings-help" role="status">
              Stored in the OS keychain (not in SQLite). You can also set{" "}
              <code className="editor-settings-code">ANTHROPIC_API_KEY</code> in the environment.
            </p>
            <input
              type="password"
              autoComplete="off"
              value={anthropicInput}
              onChange={(e) => setAnthropicInput(e.target.value)}
              placeholder="sk-ant-…"
            />
            <div className="editor-settings-inline">
              <button
                type="button"
                onClick={() => void saveAnthropicKey()}
                disabled={savingAnthropic}
              >
                {savingAnthropic ? "Saving…" : "Save"}
              </button>
              <button
                type="button"
                className="editor-settings-secondary-btn"
                onClick={() => void clearAnthropicKey()}
                disabled={savingAnthropic}
              >
                Clear
              </button>
            </div>
            <p className="editor-settings-help" role="status">
              Keychain flag: {anthropicKeyStored ? "stored" : "not stored"} · Active:{" "}
              {anthropicKeyConfigured === null
                ? "…"
                : anthropicKeyConfigured
                  ? "env or keychain OK"
                  : "missing"}
            </p>
            <button
              type="button"
              className="editor-settings-secondary-btn"
              onClick={() => void refreshAnthropicStatus()}
            >
              Refresh status
            </button>
          </section>

          <section className="editor-settings-section">
            <h3>Porcupine access key</h3>
            <p className="editor-settings-help">Picovoice access key (keychain). Required for Porcupine wake.</p>
            <input
              type="password"
              autoComplete="off"
              value={porcupineInput}
              onChange={(e) => setPorcupineInput(e.target.value)}
              placeholder="Access key"
            />
            <div className="editor-settings-inline">
              <button
                type="button"
                onClick={() => void savePorcupineKey()}
                disabled={savingPorcupine}
              >
                {savingPorcupine ? "Saving…" : "Save"}
              </button>
              <button
                type="button"
                className="editor-settings-secondary-btn"
                onClick={() => void clearPorcupineKey()}
                disabled={savingPorcupine}
              >
                Clear
              </button>
            </div>
            <p className="editor-settings-help" role="status">
              Keychain flag: {porcupineKeyStored ? "stored" : "not stored"}
            </p>
          </section>

          <section className="editor-settings-section">
            <h3>App index</h3>
            <p className="editor-settings-help" role="status">
              Indexed apps:{" "}
              {appIndexCount === null ? "…" : appIndexCount}
            </p>
          </section>

          <section className="editor-settings-section">
            <h3>AI mode</h3>
            <p className="editor-settings-help">
              Commands with a sub-prompt can run an AI preview after their action chain (model{" "}
              <code className="editor-settings-code">claude-haiku-4-5</code>).
            </p>
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
