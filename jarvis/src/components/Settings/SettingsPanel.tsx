import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import {
  normalizeSttProvider,
  normalizeThemeValue,
  parseRemoteSttTimeoutSecs,
  parseThresholdSettingValue,
  validateHotkeyInput,
  type EditorTheme,
  type SttProvider,
} from "../editor/SettingsPanel.logic";
import { useSettingsStore } from "../../store/settingsStore";

const HOTKEY_KEY = "hotkey";
const THEME_KEY = "theme";
const DEFAULT_THRESHOLD_KEY = "default_fuzzy_threshold_pct";

type AppSettingsPayload = {
  porcupineKeyStored: boolean;
  wakeEngine: string;
  owwThreshold: number;
  sttProvider: string;
  remoteSttUrl: string;
  remoteSttModel: string | null;
  remoteSttTimeoutSecs: number;
  remoteSttKeyStored: boolean;
  localWhisperUseGpu: boolean;
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

  const [wakeEngine, setWakeEngine] = useState("oww");
  const [owwThreshold, setOwwThreshold] = useState(0.5);
  const [porcupineKeyStored, setPorcupineKeyStored] = useState(false);

  const [porcupineInput, setPorcupineInput] = useState("");
  const [savingPorcupine, setSavingPorcupine] = useState(false);

  const [sttProvider, setSttProvider] = useState<SttProvider>("local");
  const [remoteSttUrl, setRemoteSttUrl] = useState("");
  const [remoteSttModel, setRemoteSttModel] = useState("");
  const [remoteSttTimeoutSecs, setRemoteSttTimeoutSecs] = useState(30);
  const [remoteSttKeyStored, setRemoteSttKeyStored] = useState(false);
  const [remoteSttKeyInput, setRemoteSttKeyInput] = useState("");
  const [savingRemoteStt, setSavingRemoteStt] = useState(false);
  const [localWhisperUseGpu, setLocalWhisperUseGpu] = useState(false);
  const [whisperGpuCompileSupported, setWhisperGpuCompileSupported] = useState(false);

  const refreshFromBackend = async () => {
    const [savedHotkey, savedThreshold, savedTheme, app, gpuSupported] = await Promise.all([
      invoke<string | null>("get_setting", { key: HOTKEY_KEY }),
      invoke<string | null>("get_setting", { key: DEFAULT_THRESHOLD_KEY }),
      invoke<string | null>("get_setting", { key: THEME_KEY }),
      invoke<AppSettingsPayload>("get_settings"),
      invoke<boolean>("whisper_gpu_compile_supported"),
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
    setWakeEngine(app.wakeEngine);
    setOwwThreshold(app.owwThreshold);
    setPorcupineKeyStored(app.porcupineKeyStored);
    setSttProvider(normalizeSttProvider(app.sttProvider));
    setRemoteSttUrl(app.remoteSttUrl ?? "");
    setRemoteSttModel(app.remoteSttModel ?? "");
    setRemoteSttTimeoutSecs(app.remoteSttTimeoutSecs);
    setRemoteSttKeyStored(app.remoteSttKeyStored);
    setLocalWhisperUseGpu(app.localWhisperUseGpu);
    setWhisperGpuCompileSupported(gpuSupported);
  };

  useEffect(() => {
    let mounted = true;
    const load = async () => {
      try {
        await refreshFromBackend();
      } catch (err) {
        if (!mounted) return;
        setToastText(`Failed to load settings: ${String(err)}`);
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
    } catch (err) {
      setToastText(`Failed to save wake engine: ${String(err)}`);
    }
  };

  const persistSttProvider = async (next: SttProvider) => {
    setSttProvider(next);
    try {
      const s = await invoke<AppSettingsPayload>("update_settings", {
        patch: { sttProvider: next },
      });
      setSttProvider(normalizeSttProvider(s.sttProvider));
      setRemoteSttUrl(s.remoteSttUrl ?? "");
      setRemoteSttModel(s.remoteSttModel ?? "");
      setRemoteSttTimeoutSecs(s.remoteSttTimeoutSecs);
      setRemoteSttKeyStored(s.remoteSttKeyStored);
      setLocalWhisperUseGpu(s.localWhisperUseGpu);
    } catch (err) {
      setToastText(`Failed to save transcription provider: ${String(err)}`);
    }
  };

  const persistLocalWhisperUseGpu = async (next: boolean) => {
    setLocalWhisperUseGpu(next);
    try {
      const s = await invoke<AppSettingsPayload>("update_settings", {
        patch: { localWhisperUseGpu: next },
      });
      setLocalWhisperUseGpu(s.localWhisperUseGpu);
    } catch (err) {
      setToastText(`Failed to save Whisper GPU setting: ${String(err)}`);
    }
  };

  const saveRemoteSttEndpoint = async () => {
    if (sttProvider !== "remote") return;
    const timeout = parseRemoteSttTimeoutSecs(String(remoteSttTimeoutSecs));
    if (timeout === null) {
      setToastText("Remote STT timeout must be between 1 and 300 seconds.");
      return;
    }
    setSavingRemoteStt(true);
    try {
      await invoke<AppSettingsPayload>("update_settings", {
        patch: {
          sttProvider: "remote",
          remoteSttUrl: remoteSttUrl.trim(),
          remoteSttModel: remoteSttModel.trim(),
          remoteSttTimeoutSecs: timeout,
        },
      });
      await refreshFromBackend();
      setToastText("Remote STT settings saved");
    } catch (err) {
      setToastText(`Failed to save remote STT settings: ${String(err)}`);
    } finally {
      setSavingRemoteStt(false);
    }
  };

  const saveRemoteSttKey = async () => {
    if (!remoteSttKeyInput.trim()) {
      setToastText("Enter an API key before saving.");
      return;
    }
    setSavingRemoteStt(true);
    try {
      await invoke("save_api_key", { service: "remote_stt", key: remoteSttKeyInput });
      setRemoteSttKeyInput("");
      await refreshFromBackend();
      setToastText("Remote STT API key saved to OS keychain");
    } catch (err) {
      setToastText(`Failed to save remote STT key: ${String(err)}`);
    } finally {
      setSavingRemoteStt(false);
    }
  };

  const clearRemoteSttKey = async () => {
    setSavingRemoteStt(true);
    try {
      await invoke("delete_api_key", { service: "remote_stt" });
      await refreshFromBackend();
      setToastText("Remote STT key cleared");
    } catch (err) {
      setToastText(`Failed to clear remote STT key: ${String(err)}`);
    } finally {
      setSavingRemoteStt(false);
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
            <h3>Transcription (speech-to-text)</h3>
            <label>
              Provider
              <select
                value={sttProvider}
                onChange={(e) =>
                  void persistSttProvider(normalizeSttProvider(e.target.value))
                }
              >
                <option value="local">Local on-device (Whisper)</option>
                <option value="os">Operating system API</option>
                <option value="remote">Remote HTTP API</option>
              </select>
            </label>
            <p className="editor-settings-help">
              Choose how spoken audio is turned into text for command matching. Remote requires a
              compatible HTTPS endpoint and API key in the keychain.
            </p>

            {sttProvider === "local" && (
              <label className="editor-settings-checkbox-row">
                <input
                  type="checkbox"
                  checked={localWhisperUseGpu}
                  disabled={!whisperGpuCompileSupported}
                  onChange={(e) => void persistLocalWhisperUseGpu(e.target.checked)}
                />
                <span>Use GPU for Whisper (when available)</span>
              </label>
            )}
            {sttProvider === "local" && !whisperGpuCompileSupported && (
              <p className="editor-settings-help">
                This build uses CPU-only Whisper. Rebuild with a GPU feature to enable acceleration,
                for example{" "}
                <code className="editor-settings-code">cargo tauri build --features whisper-vulkan</code>{" "}
                (Vulkan SDK), <code className="editor-settings-code">whisper-cuda</code> (NVIDIA +
                CUDA), or on macOS <code className="editor-settings-code">whisper-metal</code>.
              </p>
            )}

            {sttProvider === "remote" && (
              <>
                <label>
                  Endpoint URL
                  <input
                    type="url"
                    autoComplete="off"
                    placeholder="https://example.com/v1/transcribe"
                    value={remoteSttUrl}
                    onChange={(e) => setRemoteSttUrl(e.target.value)}
                  />
                </label>
                <label>
                  Model (optional)
                  <input
                    type="text"
                    autoComplete="off"
                    value={remoteSttModel}
                    onChange={(e) => setRemoteSttModel(e.target.value)}
                    placeholder="provider-specific model id"
                  />
                </label>
                <label>
                  Request timeout (seconds)
                  <input
                    type="number"
                    min={1}
                    max={300}
                    value={remoteSttTimeoutSecs}
                    onChange={(e) => setRemoteSttTimeoutSecs(Number(e.target.value))}
                  />
                </label>
                <div className="editor-settings-inline">
                  <button
                    type="button"
                    onClick={() => void saveRemoteSttEndpoint()}
                    disabled={savingRemoteStt}
                  >
                    {savingRemoteStt ? "Saving…" : "Save remote STT settings"}
                  </button>
                </div>
                <h4>Remote API key</h4>
                <p className="editor-settings-help">
                  Stored in the OS keychain; never sent to the React layer after save.
                </p>
                <input
                  type="password"
                  autoComplete="off"
                  value={remoteSttKeyInput}
                  onChange={(e) => setRemoteSttKeyInput(e.target.value)}
                  placeholder="API key"
                />
                <div className="editor-settings-inline">
                  <button
                    type="button"
                    onClick={() => void saveRemoteSttKey()}
                    disabled={savingRemoteStt}
                  >
                    {savingRemoteStt ? "Saving…" : "Save"}
                  </button>
                  <button
                    type="button"
                    className="editor-settings-secondary-btn"
                    onClick={() => void clearRemoteSttKey()}
                    disabled={savingRemoteStt}
                  >
                    Clear
                  </button>
                </div>
                <p className="editor-settings-help" role="status">
                  Keychain flag: {remoteSttKeyStored ? "stored" : "not stored"}
                </p>
              </>
            )}
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
