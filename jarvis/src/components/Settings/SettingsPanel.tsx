import { invoke } from "@tauri-apps/api/core";
import type { KeyboardEvent as ReactKeyboardEvent, RefObject } from "react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import {
  applyEditorThemeToDocument,
  normalizeSttProvider,
  normalizeThemePreference,
  parseRemoteSttTimeoutSecs,
  parseThresholdSettingValue,
  validateHotkeyInput,
  type EditorThemePreference,
  type SttProvider,
} from "../editor/SettingsPanel.logic";
import { useSettingsStore } from "../../store/settingsStore";
import { formatUserError } from "../../utils/userErrors";

const HOTKEY_KEY = "hotkey";
const THEME_KEY = "theme";
const DEFAULT_THRESHOLD_KEY = "default_fuzzy_threshold_pct";

type SettingsNavId = "general" | "speech" | "wake" | "about";

const SETTINGS_NAV: { id: SettingsNavId; label: string }[] = [
  { id: "general", label: "General" },
  { id: "speech", label: "Speech" },
  { id: "wake", label: "Wake word" },
  { id: "about", label: "About" },
];

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

function settingsFocusables(root: HTMLElement): HTMLElement[] {
  const sel =
    "button:not([disabled]), a[href], input:not([disabled]), select:not([disabled]), textarea:not([disabled])";
  return Array.from(root.querySelectorAll<HTMLElement>(sel)).filter((el) => {
    const style = window.getComputedStyle(el);
    return style.visibility !== "hidden" && style.display !== "none";
  });
}

type SettingsPanelProps = {
  onClose: () => void;
  returnFocusRef?: RefObject<HTMLElement | null>;
};

export function SettingsPanel({ onClose, returnFocusRef }: SettingsPanelProps) {
  const appIndexCount = useSettingsStore((s) => s.appIndexCount);

  const [loading, setLoading] = useState(true);
  const [hotkey, setHotkey] = useState("ctrl+shift+j");
  const [threshold, setThreshold] = useState(0.8);
  const [theme, setTheme] = useState<EditorThemePreference>("system");
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

  const panelRef = useRef<HTMLElement | null>(null);
  const hotkeyInputRef = useRef<HTMLInputElement>(null);
  const generalNavRef = useRef<HTMLButtonElement>(null);
  const [settingsNav, setSettingsNav] = useState<SettingsNavId>("general");

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
    const normalizedTheme = normalizeThemePreference(savedTheme);
    setTheme(normalizedTheme);
    applyEditorThemeToDocument(normalizedTheme);
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
        setToastText(formatUserError(err, "Could not load settings. Try again."));
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

  useLayoutEffect(() => {
    if (loading) return;
    generalNavRef.current?.focus();
  }, [loading]);

  useEffect(() => {
    const onKey = (ev: Event) => {
      const e = ev as KeyboardEvent;
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  useEffect(() => {
    const focusTarget = returnFocusRef?.current ?? null;
    return () => {
      focusTarget?.focus();
    };
  }, [returnFocusRef]);

  const onPanelKeyDown = (e: ReactKeyboardEvent<HTMLElement>) => {
    if (e.key !== "Tab" || !panelRef.current) return;
    const nodes = settingsFocusables(panelRef.current);
    if (nodes.length === 0) return;
    const first = nodes[0];
    const last = nodes[nodes.length - 1];
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  };

  const saveThreshold = async (nextThreshold: number) => {
    const pct = Math.round(nextThreshold * 100);
    try {
      await invoke("set_setting", {
        key: DEFAULT_THRESHOLD_KEY,
        value: String(pct),
      });
    } catch (err) {
      setToastText(formatUserError(err, "Could not save the default match threshold."));
    }
  };

  const saveTheme = async (nextTheme: EditorThemePreference) => {
    applyEditorThemeToDocument(nextTheme);
    setTheme(nextTheme);
    try {
      await invoke("set_setting", { key: THEME_KEY, value: nextTheme });
    } catch (err) {
      setToastText(formatUserError(err, "Could not save the color scheme."));
    }
  };

  const commitOwwThreshold = async (next: number) => {
    setOwwThreshold(next);
    try {
      await invoke<AppSettingsPayload>("update_settings", {
        patch: { owwThreshold: next },
      });
    } catch (err) {
      setToastText(formatUserError(err, "Could not save OpenWakeWord sensitivity."));
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
      setHotkeyError(formatUserError(err, "Could not save the hotkey. Try a different shortcut."));
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
      setToastText(formatUserError(err, "Could not save the wake engine."));
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
      setToastText(formatUserError(err, "Could not save the transcription provider."));
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
      setToastText(formatUserError(err, "Could not save the Whisper GPU option."));
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
      setToastText(formatUserError(err, "Could not save remote speech settings."));
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
      setToastText(formatUserError(err, "Could not save the remote speech API key."));
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
      setToastText(formatUserError(err, "Could not clear the remote speech API key."));
    } finally {
      setSavingRemoteStt(false);
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
      setToastText(formatUserError(err, "Could not save the Porcupine access key."));
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
      setToastText(formatUserError(err, "Could not clear the Porcupine access key."));
    } finally {
      setSavingPorcupine(false);
    }
  };

  return (
    <aside
      ref={panelRef}
      className="editor-settings-panel"
      role="dialog"
      aria-modal="true"
      aria-busy={loading}
      aria-label="Settings"
      onKeyDown={onPanelKeyDown}
    >
      <header className="editor-settings-header">
        <h2 id="settings-dialog-title">Settings</h2>
        <button type="button" onClick={onClose} aria-label="Close settings">
          Close
        </button>
      </header>

      {loading ? (
        <p className="editor-settings-loading" aria-live="polite">
          Loading settings…
        </p>
      ) : (
        <div className="editor-settings-body">
          <nav className="editor-settings-nav" aria-label="Settings categories">
            {SETTINGS_NAV.map((item) => (
              <button
                key={item.id}
                ref={item.id === "general" ? generalNavRef : undefined}
                type="button"
                className={`editor-settings-nav-btn${settingsNav === item.id ? " is-active" : ""}`}
                aria-current={settingsNav === item.id ? "page" : undefined}
                onClick={() => setSettingsNav(item.id)}
              >
                {item.label}
              </button>
            ))}
          </nav>
          <div className="editor-settings-pane">
            {settingsNav === "general" && (
              <div className="editor-settings-content" aria-labelledby="settings-pane-general">
                <h3 className="editor-settings-pane-heading" id="settings-pane-general">
                  General
                </h3>
          <section className="editor-settings-section">
            <h4>Hotkey</h4>
            <label>
              Global shortcut
              <div className="editor-settings-inline">
                <input
                  ref={hotkeyInputRef}
                  value={hotkey}
                  onChange={(e) => setHotkey(e.target.value)}
                  placeholder="ctrl+shift+j"
                />
                <button type="button" onClick={() => void saveHotkey()} disabled={savingHotkey}>
                  {savingHotkey ? "Saving..." : "Save"}
                </button>
              </div>
            </label>
            {hotkeyError && <p className="editor-field-error">{hotkeyError}</p>}
          </section>

          <section className="editor-settings-section">
            <h4>Default fuzzy threshold</h4>
            <label>
              {threshold.toFixed(2)}
              <input
                type="range"
                min={0.5}
                max={1}
                step={0.01}
                value={threshold}
                onChange={(e) => setThreshold(Number(e.target.value))}
                onPointerUp={(e) => void saveThreshold(Number((e.target as HTMLInputElement).value))}
                onKeyUp={(e) => {
                  if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
                  void saveThreshold(Number((e.target as HTMLInputElement).value));
                }}
              />
            </label>
            <p className="editor-settings-help">Default for new commands (you can override per command).</p>
          </section>

          <section className="editor-settings-section">
            <h4>Appearance</h4>
            <label htmlFor="editor-theme-select">
              Color scheme
              <select
                id="editor-theme-select"
                value={theme}
                onChange={(e) => void saveTheme(normalizeThemePreference(e.target.value))}
              >
                <option value="system">System</option>
                <option value="dark">Dark</option>
                <option value="light">Light</option>
              </select>
            </label>
          </section>
                </div>
            )}

            {settingsNav === "speech" && (
              <div className="editor-settings-content" aria-labelledby="settings-pane-speech">
                <h3 className="editor-settings-pane-heading" id="settings-pane-speech">
                  Speech
                </h3>
                <section className="editor-settings-section">
                  <h4>Transcription</h4>
                  <label htmlFor="editor-stt-provider">
                    Provider
                    <select
                      id="editor-stt-provider"
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
                    How spoken audio becomes text for command matching. Remote needs a compatible
                    HTTPS endpoint and an API key in the keychain.
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
                      This build uses CPU-only Whisper. Rebuild with a GPU feature to enable
                      acceleration, for example{" "}
                      <code className="editor-settings-code">
                        cargo tauri build --features whisper-vulkan
                      </code>{" "}
                      (Vulkan SDK), <code className="editor-settings-code">whisper-cuda</code>{" "}
                      (NVIDIA + CUDA), or on macOS{" "}
                      <code className="editor-settings-code">whisper-metal</code>.
                    </p>
                  )}

                  {sttProvider === "remote" && (
                    <>
                      <label htmlFor="editor-remote-stt-url">
                        Endpoint URL
                        <input
                          id="editor-remote-stt-url"
                          type="url"
                          autoComplete="off"
                          placeholder="https://example.com/v1/transcribe"
                          value={remoteSttUrl}
                          onChange={(e) => setRemoteSttUrl(e.target.value)}
                        />
                      </label>
                      <label htmlFor="editor-remote-stt-model">
                        Model (optional)
                        <input
                          id="editor-remote-stt-model"
                          type="text"
                          autoComplete="off"
                          value={remoteSttModel}
                          onChange={(e) => setRemoteSttModel(e.target.value)}
                          placeholder="provider-specific model id"
                        />
                      </label>
                      <label htmlFor="editor-remote-stt-timeout">
                        Request timeout (seconds)
                        <input
                          id="editor-remote-stt-timeout"
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
                      <label htmlFor="editor-remote-stt-key">
                        API key
                        <input
                          id="editor-remote-stt-key"
                          type="password"
                          autoComplete="off"
                          value={remoteSttKeyInput}
                          onChange={(e) => setRemoteSttKeyInput(e.target.value)}
                          placeholder="Paste API key"
                          aria-describedby="remote-stt-key-help"
                        />
                      </label>
                      <p id="remote-stt-key-help" className="editor-settings-help">
                        Saved to the OS keychain; not retained in this form after save.
                      </p>
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
              </div>
            )}

            {settingsNav === "wake" && (
              <div className="editor-settings-content" aria-labelledby="settings-pane-wake">
                <h3 className="editor-settings-pane-heading" id="settings-pane-wake">
                  Wake word
                </h3>
                <section className="editor-settings-section">
                  <h4>Wake engine</h4>
                  <label htmlFor="editor-wake-engine">
                    Engine
                    <select
                      id="editor-wake-engine"
                      value={wakeEngine}
                      onChange={(e) => void persistWakeEngine(e.target.value)}
                    >
                      <option value="hotkey">Hotkey only</option>
                      <option value="porcupine">Porcupine</option>
                      <option value="oww">OpenWakeWord</option>
                    </select>
                  </label>
                  <p className="editor-settings-help">
                    Hotkey-only matches earlier phases. Porcupine and OpenWakeWord need models and
                    keys as documented in the README.
                  </p>
                </section>

                {wakeEngine === "oww" && (
                  <section className="editor-settings-section">
                    <h4>OpenWakeWord sensitivity</h4>
                    <label>
                      {owwThreshold.toFixed(2)}
                      <input
                        type="range"
                        min={0.01}
                        max={1}
                        step={0.01}
                        value={owwThreshold}
                        onChange={(e) => setOwwThreshold(Number(e.target.value))}
                        onPointerUp={(e) =>
                          void commitOwwThreshold(Number((e.target as HTMLInputElement).value))
                        }
                        onKeyUp={(e) => {
                          if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
                          void commitOwwThreshold(Number((e.target as HTMLInputElement).value));
                        }}
                      />
                    </label>
                    <p className="editor-settings-help">
                      Higher values require a clearer wake phrase match before listening starts.
                    </p>
                  </section>
                )}

                {wakeEngine === "porcupine" && (
                  <section className="editor-settings-section">
                    <h4>Porcupine access key</h4>
                    <p className="editor-settings-help">
                      Picovoice access key (keychain). Required for Porcupine wake.
                    </p>
                    <label htmlFor="editor-porcupine-key">
                      Access key
                      <input
                        id="editor-porcupine-key"
                        type="password"
                        autoComplete="off"
                        value={porcupineInput}
                        onChange={(e) => setPorcupineInput(e.target.value)}
                        placeholder="Paste access key"
                        aria-describedby="porcupine-key-help"
                      />
                    </label>
                    <p id="porcupine-key-help" className="editor-settings-help">
                      Stored in the OS keychain; cleared from this field after save.
                    </p>
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
                )}

                {wakeEngine === "hotkey" && (
                  <p className="editor-settings-help">
                    Wake word detection is off. Use the global hotkey to start listening.
                  </p>
                )}
              </div>
            )}

            {settingsNav === "about" && (
              <div className="editor-settings-content" aria-labelledby="settings-pane-about">
                <h3 className="editor-settings-pane-heading" id="settings-pane-about">
                  About
                </h3>
                <section className="editor-settings-section">
                  <h4>App index</h4>
                  <p className="editor-settings-help" role="status">
                    Indexed apps: {appIndexCount === null ? "…" : appIndexCount}
                  </p>
                </section>
              </div>
            )}
          </div>
        </div>
      )}

      {toastText && (
        <div className="editor-inline-toast editor-settings-toast" role="status">
          {toastText}
        </div>
      )}
    </aside>
  );
}
