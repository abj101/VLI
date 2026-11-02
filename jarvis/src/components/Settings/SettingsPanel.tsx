import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  applyEditorThemeToDocument,
  applyEditorTransparencyToDocument,
  applyHudTransparencyToDocument,
  normalizeDictationHotkeyMode,
  normalizeDictationStyle,
  DEFAULT_DICTATION_CLEANUP,
  DEFAULT_DICTATION_STYLE,
  DEFAULT_DICTATION_VOCABULARY,
  DICTATION_STYLE_OPTIONS,
  DEFAULT_LOCAL_WHISPER_MODEL,
  normalizeLocalWhisperModel,
  LOCAL_WHISPER_MODEL_OPTIONS,
  normalizeThemePreference,
  parseEditorTransparencySettingValue,
  parseHudTransparencySettingValue,
  EDITOR_TRANSPARENCY_DEFAULT,
  HUD_TRANSPARENCY_DEFAULT,
  validateHotkeyInput,
  scheduleSettingsNoticeAutoDismiss,
  scheduleSettingsNoticeFadeOut,
  SETTINGS_NOTICE_FADE_MS,
  type SettingsNoticePhase,
  createHotkeyRecordingState,
  processHotkeyRecordingKeyDown,
  processHotkeyRecordingKeyUp,
  type HotkeyRecordingState,
  type EditorThemePreference,
  type DictationHotkeyMode,
  type DictationStyle,
  type LocalWhisperModelId,
} from "../editor/SettingsPanel.logic";
import { formatUserError } from "../../utils/userErrors";
import type { EditorSettingsNavId } from "./settingsNav";
import { AppIndexPane } from "./AppIndexPane";
import { WhisperModelsPane } from "./WhisperModelsPane";
import { HotkeyChordDisplay } from "./HotkeyChordDisplay";
import { SettingsLabelWithInfo } from "./SettingsInfoTip";
import { EditorSelect } from "../ui/EditorSelect";
import { EditorSpinner } from "../editor/EditorSpinner";

const HOTKEY_KEY = "hotkey";
const THEME_KEY = "theme";
const HUD_TRANSPARENCY_KEY = "hud_transparency";
const EDITOR_TRANSPARENCY_KEY = "editor_transparency";

type AppSettingsPayload = {
  wakeEngine: string;
  owwThreshold: number;
  snapToggleEnabled: boolean;
  snapSensitivityDb: number;
  sttProvider: string;
  remoteSttUrl: string;
  remoteSttModel: string | null;
  remoteSttTimeoutSecs: number;
  remoteSttKeyStored: boolean;
  localWhisperUseGpu: boolean;
  localWhisperModel: string;
  llmRouterModelPath: string | null;
  llmRouterConfidenceThreshold: number;
  llmRouterTier2Enabled: boolean;
  llmRouterWarmupOnLaunch: boolean;
  dictationHotkey: string;
  dictationHotkeyMode: string;
  dictationCleanup: boolean;
  dictationStyle: string;
  dictationVocabulary: string;
};

type RouterStatusPayload = {
  runtimeAvailable: boolean;
  modelPresent: boolean;
  modelPath: string | null;
  tier2Enabled: boolean;
  confidenceThreshold: number;
  message: string | null;
};

type RouterWarmupPayload = {
  ready: boolean;
  message: string;
};

type WhisperModelStatusPayload = {
  model: string;
  available: boolean;
};

type SettingsPanelProps = {
  /** Always embedded in the editor shell; accepted so the existing call site compiles. */
  embedded?: boolean;
  /** Pane to show, controlled by the editor sidebar. */
  activeNav?: EditorSettingsNavId;
};

export function SettingsPanel({ activeNav: pane = "hotkeys" }: SettingsPanelProps) {
  const [loading, setLoading] = useState(true);
  const [hotkey, setHotkey] = useState("ctrl+shift+j");
  const [theme, setTheme] = useState<EditorThemePreference>("system");
  const [hudTransparency, setHudTransparency] = useState(HUD_TRANSPARENCY_DEFAULT);
  const [editorTransparency, setEditorTransparency] = useState(EDITOR_TRANSPARENCY_DEFAULT);
  const [hotkeyError, setHotkeyError] = useState<string | null>(null);
  const [savingHotkey, setSavingHotkey] = useState(false);
  const [hotkeyRecording, setHotkeyRecording] = useState(false);
  const [hotkeyCapturedThisSession, setHotkeyCapturedThisSession] = useState(false);

  const [dictationHotkey, setDictationHotkey] = useState("ctrl+shift+d");
  const [dictationHotkeyMode, setDictationHotkeyMode] = useState<DictationHotkeyMode>("toggle");
  const [dictationCleanup, setDictationCleanup] = useState(DEFAULT_DICTATION_CLEANUP);
  const [dictationStyle, setDictationStyle] = useState<DictationStyle>(DEFAULT_DICTATION_STYLE);
  const [dictationVocabulary, setDictationVocabulary] = useState(DEFAULT_DICTATION_VOCABULARY);
  const [dictationHotkeyError, setDictationHotkeyError] = useState<string | null>(null);
  const [savingDictationHotkey, setSavingDictationHotkey] = useState(false);
  const [dictationHotkeyRecording, setDictationHotkeyRecording] = useState(false);
  const [dictationHotkeyCapturedThisSession, setDictationHotkeyCapturedThisSession] =
    useState(false);

  const [wakeEngine, setWakeEngine] = useState("oww");
  const [owwThreshold, setOwwThreshold] = useState(0.7);
  const [snapToggleEnabled, setSnapToggleEnabled] = useState(false);
  const [snapSensitivityDb, setSnapSensitivityDb] = useState(25);

  const [localWhisperUseGpu, setLocalWhisperUseGpu] = useState(false);
  const [localWhisperModel, setLocalWhisperModel] = useState(DEFAULT_LOCAL_WHISPER_MODEL);
  const [llmRouterTier2Enabled, setLlmRouterTier2Enabled] = useState(false);
  const [routerPreparing, setRouterPreparing] = useState(false);
  const [routerPrepMessage, setRouterPrepMessage] = useState<string | null>(null);
  const [routerStatus, setRouterStatus] = useState<RouterStatusPayload>({
    runtimeAvailable: false,
    modelPresent: false,
    modelPath: null,
    tier2Enabled: false,
    confidenceThreshold: 0.7,
    message: "LLM router is not available in this build.",
  });

  const hotkeyRecordingStateRef = useRef<HotkeyRecordingState>(createHotkeyRecordingState());
  const hotkeyBeforeRecordingRef = useRef<string | null>(null);
  const dictationHotkeyRecordingStateRef = useRef<HotkeyRecordingState>(createHotkeyRecordingState());
  const dictationHotkeyBeforeRecordingRef = useRef<string | null>(null);

  const [noticeText, setNoticeText] = useState<string | null>(null);
  const [noticePhase, setNoticePhase] = useState<SettingsNoticePhase>("hidden");
  const noticeFadeCancelRef = useRef<(() => void) | null>(null);
  const noticeTextRef = useRef<string | null>(null);
  const paneRef = useRef(pane);

  const clearNoticeFadeTimer = useCallback(() => {
    noticeFadeCancelRef.current?.();
    noticeFadeCancelRef.current = null;
  }, []);

  const finishNoticeHide = useCallback(() => {
    clearNoticeFadeTimer();
    setNoticePhase("hidden");
    setNoticeText(null);
    noticeTextRef.current = null;
  }, [clearNoticeFadeTimer]);

  const dismissNotice = useCallback(() => {
    if (!noticeTextRef.current) return;
    clearNoticeFadeTimer();
    setNoticePhase("hiding");
    noticeFadeCancelRef.current = scheduleSettingsNoticeFadeOut(() => {
      noticeFadeCancelRef.current = null;
      finishNoticeHide();
    }, SETTINGS_NOTICE_FADE_MS);
  }, [clearNoticeFadeTimer, finishNoticeHide]);

  const showSettingsNotice = useCallback((text: string) => {
    clearNoticeFadeTimer();
    noticeTextRef.current = text;
    setNoticeText(text);
    setNoticePhase("hidden");
    requestAnimationFrame(() => {
      setNoticePhase("visible");
    });
  }, [clearNoticeFadeTimer]);

  useEffect(() => {
    if (noticePhase !== "visible" || !noticeText) return;
    return scheduleSettingsNoticeAutoDismiss(() => {
      dismissNotice();
    });
  }, [noticePhase, noticeText, dismissNotice]);

  useEffect(() => {
    return () => clearNoticeFadeTimer();
  }, [clearNoticeFadeTimer]);

  useEffect(() => {
    if (paneRef.current === pane) return;
    paneRef.current = pane;
    if (noticeTextRef.current) {
      dismissNotice();
    }
  }, [pane, dismissNotice]);

  const refreshFromBackend = async () => {
    const [
      savedHotkey,
      savedTheme,
      savedHudTransparency,
      savedEditorTransparency,
      app,
          router,
    ] = await Promise.all([
        invoke<string | null>("get_setting", { key: HOTKEY_KEY }),
        invoke<string | null>("get_setting", { key: THEME_KEY }),
        invoke<string | null>("get_setting", { key: HUD_TRANSPARENCY_KEY }),
        invoke<string | null>("get_setting", { key: EDITOR_TRANSPARENCY_KEY }),
        invoke<AppSettingsPayload>("get_settings"),
        invoke<RouterStatusPayload>("router_status"),
      ]);
    if (savedHotkey && savedHotkey.trim().length > 0) {
      setHotkey(savedHotkey.trim());
    }
    const normalizedTheme = normalizeThemePreference(savedTheme);
    setTheme(normalizedTheme);
    applyEditorThemeToDocument(normalizedTheme);
    const parsedHudTransparency = parseHudTransparencySettingValue(savedHudTransparency);
    setHudTransparency(parsedHudTransparency);
    applyHudTransparencyToDocument(parsedHudTransparency, normalizedTheme);
    const parsedEditorTransparency = parseEditorTransparencySettingValue(savedEditorTransparency);
    setEditorTransparency(parsedEditorTransparency);
    applyEditorTransparencyToDocument(parsedEditorTransparency, normalizedTheme);
    setWakeEngine(app.wakeEngine);
    setOwwThreshold(app.owwThreshold);
    setSnapToggleEnabled(app.snapToggleEnabled);
    setSnapSensitivityDb(app.snapSensitivityDb);
    setLocalWhisperUseGpu(app.localWhisperUseGpu);
    setLocalWhisperModel(normalizeLocalWhisperModel(app.localWhisperModel));
      setLlmRouterTier2Enabled(app.llmRouterTier2Enabled);
    setRouterStatus(router);
    setDictationHotkey(
      app.dictationHotkey?.trim().length ? app.dictationHotkey.trim() : "ctrl+shift+d",
    );
    setDictationHotkeyMode(normalizeDictationHotkeyMode(app.dictationHotkeyMode));
    setDictationCleanup(app.dictationCleanup ?? DEFAULT_DICTATION_CLEANUP);
    setDictationStyle(normalizeDictationStyle(app.dictationStyle));
    setDictationVocabulary(app.dictationVocabulary ?? DEFAULT_DICTATION_VOCABULARY);
  };

  const routerModelPresent = routerStatus.modelPresent;
  const routerRuntimeReady = routerModelPresent;
  const routerTier2StatusMessage = routerPreparing
    ? (routerPrepMessage ?? "Preparing model…")
    : (routerPrepMessage ??
      (!routerModelPresent
        ? "Router model missing — run npm run fetch-models from the jarvis folder."
        : routerStatus.message));

  useEffect(() => {
    let mounted = true;
    const load = async () => {
      try {
        await refreshFromBackend();
      } catch (err) {
        if (!mounted) return;
        showSettingsNotice(formatUserError(err, "Could not load settings. Try again."));
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

  useEffect(() => {
    void invoke("set_hotkey_recording", {
      recording: hotkeyRecording || dictationHotkeyRecording,
    });
    return () => {
      void invoke("set_hotkey_recording", { recording: false });
    };
  }, [hotkeyRecording, dictationHotkeyRecording]);

  const cancelHotkeyRecording = useCallback(() => {
    if (hotkeyBeforeRecordingRef.current != null) {
      setHotkey(hotkeyBeforeRecordingRef.current);
    }
    hotkeyBeforeRecordingRef.current = null;
    setHotkeyCapturedThisSession(false);
    setHotkeyRecording(false);
  }, []);

  useEffect(() => {
    if (!hotkeyRecording) return;

    hotkeyRecordingStateRef.current = createHotkeyRecordingState();

    const onKeyDown = (ev: Event) => {
      const e = ev as KeyboardEvent;
      const { next, action } = processHotkeyRecordingKeyDown(e, hotkeyRecordingStateRef.current);
      hotkeyRecordingStateRef.current = next;

      if (action.type === "ignore") return;

      e.preventDefault();
      e.stopPropagation();

      if (action.type === "cancel") {
        cancelHotkeyRecording();
        return;
      }
      if (action.type === "clear") {
        setHotkey("");
        setHotkeyCapturedThisSession(true);
        return;
      }
      setHotkey(action.chord);
      setHotkeyCapturedThisSession(true);
    };

    const onKeyUp = (ev: Event) => {
      const e = ev as KeyboardEvent;
      hotkeyRecordingStateRef.current = processHotkeyRecordingKeyUp(
        e,
        hotkeyRecordingStateRef.current,
      );
    };

    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("keyup", onKeyUp, true);
    return () => {
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("keyup", onKeyUp, true);
    };
  }, [hotkeyRecording, cancelHotkeyRecording]);

  const cancelDictationHotkeyRecording = useCallback(() => {
    if (dictationHotkeyBeforeRecordingRef.current != null) {
      setDictationHotkey(dictationHotkeyBeforeRecordingRef.current);
    }
    dictationHotkeyBeforeRecordingRef.current = null;
    setDictationHotkeyCapturedThisSession(false);
    setDictationHotkeyRecording(false);
  }, []);

  useEffect(() => {
    if (!dictationHotkeyRecording) return;

    dictationHotkeyRecordingStateRef.current = createHotkeyRecordingState();

    const onKeyDown = (ev: Event) => {
      const e = ev as KeyboardEvent;
      const { next, action } = processHotkeyRecordingKeyDown(
        e,
        dictationHotkeyRecordingStateRef.current,
      );
      dictationHotkeyRecordingStateRef.current = next;

      if (action.type === "ignore") return;

      e.preventDefault();
      e.stopPropagation();

      if (action.type === "cancel") {
        cancelDictationHotkeyRecording();
        return;
      }
      if (action.type === "clear") {
        setDictationHotkey("");
        setDictationHotkeyCapturedThisSession(true);
        return;
      }
      setDictationHotkey(action.chord);
      setDictationHotkeyCapturedThisSession(true);
    };

    const onKeyUp = (ev: Event) => {
      const e = ev as KeyboardEvent;
      dictationHotkeyRecordingStateRef.current = processHotkeyRecordingKeyUp(
        e,
        dictationHotkeyRecordingStateRef.current,
      );
    };

    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("keyup", onKeyUp, true);
    return () => {
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("keyup", onKeyUp, true);
    };
  }, [dictationHotkeyRecording, cancelDictationHotkeyRecording]);

  const saveTheme = async (nextTheme: EditorThemePreference) => {
    applyEditorThemeToDocument(nextTheme);
    applyHudTransparencyToDocument(hudTransparency, nextTheme);
    applyEditorTransparencyToDocument(editorTransparency, nextTheme);
    setTheme(nextTheme);
    try {
      await invoke("set_setting", { key: THEME_KEY, value: nextTheme });
    } catch (err) {
      showSettingsNotice(formatUserError(err, "Could not save the color scheme."));
    }
  };

  const saveHudTransparency = async (nextTransparency: number) => {
    const clamped = parseHudTransparencySettingValue(String(nextTransparency));
    setHudTransparency(clamped);
    applyHudTransparencyToDocument(clamped, theme);
    try {
      await invoke("set_setting", {
        key: HUD_TRANSPARENCY_KEY,
        value: String(clamped),
      });
    } catch (err) {
      showSettingsNotice(formatUserError(err, "Could not save HUD transparency."));
    }
  };

  const saveEditorTransparency = async (nextTransparency: number) => {
    const clamped = parseEditorTransparencySettingValue(String(nextTransparency));
    setEditorTransparency(clamped);
    applyEditorTransparencyToDocument(clamped, theme);
    try {
      await invoke("set_setting", {
        key: EDITOR_TRANSPARENCY_KEY,
        value: String(clamped),
      });
    } catch (err) {
      showSettingsNotice(formatUserError(err, "Could not save editor transparency."));
    }
  };

  const commitOwwThreshold = async (next: number) => {
    const clamped = Math.max(0.01, Math.min(1, next));
    setOwwThreshold(clamped);
    try {
      await invoke<AppSettingsPayload>("update_settings", {
        patch: { owwThreshold: clamped },
      });
    } catch (err) {
      showSettingsNotice(formatUserError(err, "Could not save OpenWakeWord sensitivity."));
    }
  };

  const persistSnapToggleEnabled = async (next: boolean) => {
    const previous = snapToggleEnabled;
    setSnapToggleEnabled(next);
    try {
      const settings = await invoke<AppSettingsPayload>("update_settings", {
        patch: { snapToggleEnabled: next },
      });
      setSnapToggleEnabled(settings.snapToggleEnabled);
    } catch (err) {
      setSnapToggleEnabled(previous);
      showSettingsNotice(formatUserError(err, "Could not save snap toggle setting."));
    }
  };
  const commitSnapSensitivityDb = async (next: number) => {
    const clamped = Math.max(10, Math.min(30, next));
    setSnapSensitivityDb(clamped);
    try {
      const settings = await invoke<AppSettingsPayload>("update_settings", {
        patch: { snapSensitivityDb: clamped },
      });
      setSnapSensitivityDb(settings.snapSensitivityDb);
    } catch (err) {
      showSettingsNotice(formatUserError(err, "Could not save snap sensitivity."));
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
      showSettingsNotice("Hotkey updated");
    } catch (err) {
      setHotkeyError(formatUserError(err, "Could not save the hotkey. Try a different shortcut."));
    } finally {
      setSavingHotkey(false);
    }
  };

  const toggleHotkeyRecording = () => {
    if (hotkeyRecording) {
      setHotkeyRecording(false);
      if (hotkeyCapturedThisSession) {
        void saveHotkey();
      }
      return;
    }
    setHotkeyError(null);
    setHotkeyCapturedThisSession(false);
    hotkeyBeforeRecordingRef.current = hotkey;
    setHotkeyRecording(true);
  };

  const saveDictationHotkey = async () => {
    const maybeError = validateHotkeyInput(dictationHotkey);
    if (maybeError) {
      setDictationHotkeyError(maybeError);
      return;
    }
    setDictationHotkeyError(null);
    setSavingDictationHotkey(true);
    try {
      const savedHotkey = await invoke<string>("set_dictation_hotkey", {
        hotkey: dictationHotkey,
      });
      setDictationHotkey(savedHotkey);
      showSettingsNotice("Dictation hotkey updated");
    } catch (err) {
      setDictationHotkeyError(
        formatUserError(err, "Could not save the dictation hotkey. Try a different shortcut."),
      );
    } finally {
      setSavingDictationHotkey(false);
    }
  };

  const toggleDictationHotkeyRecording = () => {
    if (dictationHotkeyRecording) {
      setDictationHotkeyRecording(false);
      if (dictationHotkeyCapturedThisSession) {
        void saveDictationHotkey();
      }
      return;
    }
    setDictationHotkeyError(null);
    setDictationHotkeyCapturedThisSession(false);
    dictationHotkeyBeforeRecordingRef.current = dictationHotkey;
    setDictationHotkeyRecording(true);
  };

  const persistDictationHotkeyMode = async (next: DictationHotkeyMode) => {
    const prev = dictationHotkeyMode;
    setDictationHotkeyMode(next);
    try {
      const stored = await invoke<string>("set_dictation_hotkey_mode", { mode: next });
      setDictationHotkeyMode(normalizeDictationHotkeyMode(stored));
      showSettingsNotice(
        next === "push_to_talk" ? "Dictation set to push-to-talk" : "Dictation set to toggle",
      );
    } catch (err) {
      setDictationHotkeyMode(prev);
      showSettingsNotice(formatUserError(err, "Could not save dictation hotkey mode."));
    }
  };

  const persistDictationCleanup = async (next: boolean) => {
    const previous = dictationCleanup;
    setDictationCleanup(next);
    try {
      const settings = await invoke<AppSettingsPayload>("update_settings", {
        patch: { dictationCleanup: next },
      });
      setDictationCleanup(settings.dictationCleanup ?? DEFAULT_DICTATION_CLEANUP);
    } catch (err) {
      setDictationCleanup(previous);
      showSettingsNotice(formatUserError(err, "Could not save dictation cleanup setting."));
    }
  };

  const persistDictationStyle = async (next: DictationStyle) => {
    const previous = dictationStyle;
    setDictationStyle(next);
    try {
      const settings = await invoke<AppSettingsPayload>("update_settings", {
        patch: { dictationStyle: next },
      });
      setDictationStyle(normalizeDictationStyle(settings.dictationStyle));
    } catch (err) {
      setDictationStyle(previous);
      showSettingsNotice(formatUserError(err, "Could not save dictation style."));
    }
  };

  const persistDictationVocabulary = async () => {
    try {
      const settings = await invoke<AppSettingsPayload>("update_settings", {
        patch: { dictationVocabulary },
      });
      setDictationVocabulary(settings.dictationVocabulary ?? DEFAULT_DICTATION_VOCABULARY);
    } catch (err) {
      showSettingsNotice(formatUserError(err, "Could not save personal vocabulary."));
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
      showSettingsNotice(formatUserError(err, "Could not save the wake engine."));
    }
  };

  const persistLlmRouterTier2 = async (next: boolean) => {
    const prev = llmRouterTier2Enabled;
    setLlmRouterTier2Enabled(next);
    if (!next) {
      setRouterPreparing(false);
      setRouterPrepMessage(null);
    } else if (routerRuntimeReady) {
      setRouterPreparing(true);
      setRouterPrepMessage("Warming router model…");
    }
    try {
      const s = await invoke<AppSettingsPayload>("update_settings", {
        patch: { llmRouterTier2Enabled: next },
      });
      setLlmRouterTier2Enabled(s.llmRouterTier2Enabled);
      if (next && s.llmRouterTier2Enabled && !routerRuntimeReady) {
        showSettingsNotice(
          "Intelligence enabled. Download the language model to use it.",
        );
      }
      if (next && s.llmRouterTier2Enabled && routerRuntimeReady) {
        const unlisten = await listen<RouterWarmupPayload>("router-warmup", (event) => {
          setRouterPrepMessage(event.payload.message);
          showSettingsNotice(event.payload.message);
          setRouterPreparing(false);
          void unlisten();
        });
        const warmup = await invoke<RouterWarmupPayload>("router_warmup");
        setRouterPrepMessage(warmup.message);
        if (warmup.ready) {
          showSettingsNotice(warmup.message);
          setRouterPreparing(false);
          void unlisten();
        }
      }
      const status = await invoke<RouterStatusPayload>("router_status");
      setRouterStatus(status);
    } catch (err) {
      setLlmRouterTier2Enabled(prev);
      showSettingsNotice(formatUserError(err, "Could not save the LLM router option."));
      setRouterPrepMessage(null);
    } finally {
      if (next) {
        setRouterPreparing(false);
      }
    }
  };

  const persistLocalWhisperModel = async (next: LocalWhisperModelId) => {
    const normalized = normalizeLocalWhisperModel(next);
    const prev = localWhisperModel;
    setLocalWhisperModel(normalized);
    try {
      const s = await invoke<AppSettingsPayload>("update_settings", {
        patch: { localWhisperModel: normalized },
      });
      setLocalWhisperModel(normalizeLocalWhisperModel(s.localWhisperModel));
      const status = await invoke<WhisperModelStatusPayload>("whisper_model_status");
      if (status.available) {
        showSettingsNotice(
          `Speech model set to ${LOCAL_WHISPER_MODEL_OPTIONS.find((o) => o.value === normalized)?.label ?? normalized}. Loads on next listen.`,
        );
      } else {
        await invoke("download_whisper_model", { modelId: normalized });
        showSettingsNotice(`Downloading ${normalized}…`);
      }
    } catch (err) {
      setLocalWhisperModel(prev);
      showSettingsNotice(formatUserError(err, "Could not save the Whisper model."));
    }
  };

  const persistLocalWhisperUseGpu = async (next: boolean) => {
    const prev = localWhisperUseGpu;
    setLocalWhisperUseGpu(next);
    try {
      const settings = await invoke<AppSettingsPayload>("update_settings", {
        patch: { localWhisperUseGpu: next },
      });
      setLocalWhisperUseGpu(settings.localWhisperUseGpu);
    } catch (err) {
      setLocalWhisperUseGpu(prev);
      showSettingsNotice(formatUserError(err, "Could not save Whisper GPU option."));
    }
  };

  return (
    <aside className="editor-settings-embedded" aria-busy={loading}>
      {loading ? (
        <p className="editor-settings-loading" aria-live="polite">
          Loading settings…
        </p>
      ) : (
        <div className="editor-settings-body">
          <div className="editor-settings-pane editor-settings-pane--solo">
            {pane === "hotkeys" && (
              <div
                className="editor-settings-content editor-settings-content--hotkeys"
                aria-labelledby={`editor-settings-nav-${pane}`}
              >
                <section className="editor-settings-section">
                  <p className="editor-settings-group-label">Voice overlay</p>

                  <div
                    className={`editor-hotkey-panel${hotkeyRecording ? " editor-hotkey-panel--recording" : ""}`}
                  >
                    <div className="editor-hotkey-panel-body">
                      <span className="editor-hotkey-panel-label">
                        <SettingsLabelWithInfo
                          tipId="tip-global-hotkey"
                          tip="Opens and closes the voice overlay from anywhere on your desktop."
                        >
                          Global shortcut
                        </SettingsLabelWithInfo>
                      </span>
                      <HotkeyChordDisplay chord={hotkey} recording={hotkeyRecording} />
                    </div>
                    <button
                      type="button"
                      className={`editor-btn editor-btn--primary editor-hotkey-record-btn${hotkeyRecording ? " editor-hotkey-record-btn--active" : ""}`}
                      onClick={toggleHotkeyRecording}
                      disabled={savingHotkey}
                      aria-pressed={hotkeyRecording}
                    >
                      {savingHotkey ? "Saving…" : hotkeyRecording ? "Stop" : "Record"}
                    </button>
                  </div>

                  {hotkeyError && <p className="editor-field-error">{hotkeyError}</p>}
                  <p className="editor-settings-help">
                    Click Record, press the shortcut, then Stop. Escape cancels; Backspace clears
                    while recording.
                  </p>
                </section>

                <section className="editor-settings-section">
                  <p className="editor-settings-group-label">Dictation</p>

                  <div
                    className={`editor-hotkey-panel${dictationHotkeyRecording ? " editor-hotkey-panel--recording" : ""}`}
                  >
                    <div className="editor-hotkey-panel-body">
                      <span className="editor-hotkey-panel-label">
                        <SettingsLabelWithInfo
                          tipId="tip-dictation-hotkey"
                          tip="Starts and stops voice dictation into the focused text field without opening the command overlay."
                        >
                          Dictation shortcut
                        </SettingsLabelWithInfo>
                      </span>
                      <HotkeyChordDisplay
                        chord={dictationHotkey}
                        recording={dictationHotkeyRecording}
                      />
                    </div>
                    <button
                      type="button"
                      className={`editor-btn editor-btn--primary editor-hotkey-record-btn${dictationHotkeyRecording ? " editor-hotkey-record-btn--active" : ""}`}
                      onClick={toggleDictationHotkeyRecording}
                      disabled={savingDictationHotkey}
                      aria-pressed={dictationHotkeyRecording}
                    >
                      {savingDictationHotkey
                        ? "Saving…"
                        : dictationHotkeyRecording
                          ? "Stop"
                          : "Record"}
                    </button>
                  </div>

                  {dictationHotkeyError && (
                    <p className="editor-field-error">{dictationHotkeyError}</p>
                  )}

                  <div className="editor-settings-row">
                    <span
                      className="editor-settings-row-label"
                      id="editor-dictation-hotkey-mode-label"
                    >
                      <SettingsLabelWithInfo
                        tipId="tip-dictation-hotkey-mode"
                        tip="Toggle starts and stops on each press. Push-to-talk dictates only while the shortcut is held."
                      >
                        Activation mode
                      </SettingsLabelWithInfo>
                    </span>
                    <span className="editor-settings-row-control">
                      <EditorSelect
                        id="editor-dictation-hotkey-mode"
                        labelledBy="editor-dictation-hotkey-mode-label"
                        value={dictationHotkeyMode}
                        onChange={(v) =>
                          void persistDictationHotkeyMode(normalizeDictationHotkeyMode(v))
                        }
                        options={[
                          { value: "toggle", label: "Toggle" },
                          { value: "push_to_talk", label: "Push to talk" },
                        ]}
                      />
                    </span>
                  </div>
                  <div className="editor-settings-row editor-settings-row--switch">
                    <span className="editor-settings-row-label editor-settings-row-label--stack" id="editor-dictation-cleanup-label">
                      <SettingsLabelWithInfo
                        tipId="tip-dictation-cleanup"
                        tip="Clean up each dictated chunk with S1-mini by Superwhisper. Turn this off to keep raw speech-to-text output."
                      >
                        Text cleanup
                      </SettingsLabelWithInfo>
                    </span>
                    <span className="editor-settings-row-control editor-settings-row-control--switch">
                      <button
                        type="button"
                        id="editor-dictation-cleanup"
                        className={`editor-switch${dictationCleanup ? " is-on" : ""}`}
                        role="switch"
                        aria-labelledby="editor-dictation-cleanup-label"
                        aria-checked={dictationCleanup}
                        onClick={() => void persistDictationCleanup(!dictationCleanup)}
                      >
                        <span className="editor-switch-knob" />
                      </button>
                    </span>
                  </div>
                  <p className="editor-settings-help">
                    S1-mini by Superwhisper cleans up dictated text after transcription.
                  </p>
                  <div className="editor-settings-row">
                    <span className="editor-settings-row-label" id="editor-dictation-style-label">
                      Writing style
                    </span>
                    <span className="editor-settings-row-control">
                      <EditorSelect
                        id="editor-dictation-style"
                        labelledBy="editor-dictation-style-label"
                        value={dictationStyle}
                        onChange={(value) =>
                          void persistDictationStyle(normalizeDictationStyle(value))
                        }
                        options={[...DICTATION_STYLE_OPTIONS]}
                      />
                    </span>
                  </div>
                  <label className="editor-settings-vocabulary" htmlFor="editor-dictation-vocabulary">
                    Personal vocabulary
                    <textarea
                      id="editor-dictation-vocabulary"
                      rows={3}
                      value={dictationVocabulary}
                      placeholder="Claude, Jarvis, Tauri"
                      onChange={(event) => setDictationVocabulary(event.target.value)}
                      onBlur={() => void persistDictationVocabulary()}
                    />
                  </label>
                  <p className="editor-settings-help">
                    Add names and terms separated by commas to improve speech recognition.
                  </p>
                </section>
              </div>
            )}

            {pane === "recognition" && (
              <div
                className="editor-settings-content editor-settings-content--recognition"
                aria-labelledby={`editor-settings-nav-${pane}`}
              >
                <div className="editor-settings-recognition-form">
                  <section className="editor-settings-section editor-settings-section--compact">
                    <p className="editor-settings-group-label">Transcription</p>
                        <WhisperModelsPane
                          activeModelId={normalizeLocalWhisperModel(localWhisperModel)}
                          useGpu={localWhisperUseGpu}
                          onSelectModel={persistLocalWhisperModel}
                          onNotice={showSettingsNotice}
                        />
                        <div className="editor-settings-row editor-settings-row--switch">
                          <div className="editor-settings-row-label editor-settings-row-label--stack">
                          <SettingsLabelWithInfo
                            id="editor-whisper-gpu-label"
                            tipId="tip-whisper-gpu"
                            tip="Use GPU acceleration for local speech recognition when available."
                          >
                            GPU acceleration
                          </SettingsLabelWithInfo>
                        </div>
                        <div className="editor-settings-row-control editor-settings-row-control--switch">
                            <button
                              type="button"
                              id="editor-whisper-gpu"
                              className={`editor-switch${localWhisperUseGpu ? " is-on" : ""}`}
                              role="switch"
                              aria-labelledby="editor-whisper-gpu-label"
                              aria-checked={localWhisperUseGpu}
                                              onClick={() => void persistLocalWhisperUseGpu(!localWhisperUseGpu)}
                            >
                              <span className="editor-switch-knob" />
                            </button>
                          </div>
                        </div>
                  </section>

                  <section className="editor-settings-section editor-settings-section--compact">
                    <p className="editor-settings-group-label">Intelligence</p>
                    <p className="editor-settings-help">
                      Understand requests beyond exact command phrases using an on-device model.
                    </p>
                    <div className="editor-settings-row editor-settings-row--switch">
                      <div className="editor-settings-row-label editor-settings-row-label--stack">
                        <SettingsLabelWithInfo
                          id="editor-llm-router-tier2-label"
                          tipId="tip-llm-router-tier2"
                          tip="Use an on-device model to understand natural requests."
                        >
                          Understand natural speech
                        </SettingsLabelWithInfo>
                        {routerTier2StatusMessage && (
                          <span
                            className="editor-settings-switch-meta"
                            role="status"
                            aria-live="polite"
                          >
                            {routerPreparing && <EditorSpinner />}
                            {routerTier2StatusMessage}
                          </span>
                        )}
                      </div>
                      <div className="editor-settings-row-control editor-settings-row-control--switch">
                        <button
                          type="button"
                          id="editor-llm-router-tier2"
                          className={`editor-switch${llmRouterTier2Enabled ? " is-on" : ""}`}
                          role="switch"
                          aria-labelledby="editor-llm-router-tier2-label"
                          aria-checked={llmRouterTier2Enabled}
                          disabled={routerPreparing}
                          onClick={() => void persistLlmRouterTier2(!llmRouterTier2Enabled)}
                        >
                          <span className="editor-switch-knob" />
                        </button>
                      </div>
                    </div>
                  </section>

                  <section className="editor-settings-section editor-settings-section--compact">
                    <p className="editor-settings-group-label">Wake word</p>
                    <div className="editor-settings-row">
                      <span className="editor-settings-row-label" id="editor-wake-engine-label">
                        <SettingsLabelWithInfo
                          tipId="tip-wake-engine"
                          tip="OpenWakeWord listens for a wake phrase before commands. Hotkey only starts listening from your global shortcut."
                        >
                          Engine
                        </SettingsLabelWithInfo>
                      </span>
                      <span className="editor-settings-row-control">
                        <EditorSelect
                          id="editor-wake-engine"
                          labelledBy="editor-wake-engine-label"
                          value={wakeEngine}
                          onChange={(v) => void persistWakeEngine(v)}
                          options={[
                            { value: "hotkey", label: "Hotkey only" },
                            { value: "oww", label: "OpenWakeWord" },
                          ]}
                        />
                      </span>
                    </div>

                    {wakeEngine === "oww" && (
                      <div className="editor-settings-row">
                        <div className="editor-settings-row-label">
                          <SettingsLabelWithInfo
                            tipId="tip-oww-sensitivity"
                            tip="How clearly you must say the wake phrase before listening starts. Higher values reduce false activations."
                          >
                            Sensitivity
                          </SettingsLabelWithInfo>
                        </div>
                        <div className="editor-settings-row-control editor-settings-row-control--slider">
                          <input
                            type="range"
                            min={0}
                            max={1}
                            step={0.01}
                            value={owwThreshold}
                            aria-valuenow={owwThreshold}
                            aria-valuemin={0}
                            aria-valuemax={1}
                            aria-valuetext={owwThreshold.toFixed(2)}
                            aria-describedby="tip-oww-sensitivity"
                            onChange={(e) => setOwwThreshold(Number(e.target.value))}
                            onPointerUp={(e) =>
                              void commitOwwThreshold(Number((e.target as HTMLInputElement).value))
                            }
                            onKeyUp={(e) => {
                              if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
                              void commitOwwThreshold(Number((e.target as HTMLInputElement).value));
                            }}
                          />
                          <span className="editor-settings-slider-value" aria-hidden>
                            {owwThreshold.toFixed(2)}
                          </span>
                        </div>
                      </div>
                    )}
                    <div className="editor-settings-row editor-settings-row--switch">
                      <div className="editor-settings-row-label">
                        <SettingsLabelWithInfo
                          id="editor-snap-toggle-label"
                          tipId="tip-snap-toggle"
                          tip="One finger snap starts dictation; another stops it. Requires OpenWakeWord on Windows."
                        >
                          Snap toggle
                        </SettingsLabelWithInfo>
                      </div>
                      <div className="editor-settings-row-control editor-settings-row-control--switch">
                        <button
                          type="button"
                          className={`editor-switch${snapToggleEnabled ? " is-on" : ""}`}
                          role="switch"
                          aria-labelledby="editor-snap-toggle-label"
                          aria-checked={snapToggleEnabled}
                          disabled={wakeEngine !== "oww"}
                          onClick={() => void persistSnapToggleEnabled(!snapToggleEnabled)}
                        >
                          <span className="editor-switch-knob" />
                        </button>
                      </div>
                    </div>
                    <p className="editor-settings-help">Works only with OpenWakeWord on Windows.</p>
                    <div className="editor-settings-row">
                      <div className="editor-settings-row-label">
                        <SettingsLabelWithInfo
                          id="editor-snap-sensitivity-label"
                          tipId="tip-snap-sensitivity"
                          tip="How far above background noise a snap must be. Lower values detect quieter snaps; higher values reduce false triggers."
                        >
                          Snap sensitivity
                        </SettingsLabelWithInfo>
                      </div>
                      <div className="editor-settings-row-control editor-settings-row-control--slider">
                        <input
                          type="range"
                          min={10}
                          max={30}
                          step={1}
                          value={snapSensitivityDb}
                          aria-labelledby="editor-snap-sensitivity-label"
                          aria-valuetext={`${snapSensitivityDb} dB`}
                          aria-describedby="tip-snap-sensitivity"
                          disabled={!snapToggleEnabled || wakeEngine !== "oww"}
                          onChange={(event) => setSnapSensitivityDb(Number(event.target.value))}
                          onPointerUp={(event) => void commitSnapSensitivityDb(Number(event.currentTarget.value))}
                          onKeyUp={(event) => {
                            if (["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End", "PageUp", "PageDown"].includes(event.key)) {
                              void commitSnapSensitivityDb(Number(event.currentTarget.value));
                            }
                          }}
                        />
                        <span className="editor-settings-slider-value" aria-hidden>{snapSensitivityDb} dB</span>
                      </div>
                    </div>
                  </section>
                </div>
              </div>
            )}

            {pane === "appearance" && (
              <div
                className="editor-settings-content editor-settings-content--recognition"
                aria-labelledby={`editor-settings-nav-${pane}`}
              >
                <div className="editor-settings-recognition-form">
                  <section className="editor-settings-section editor-settings-section--compact">
                    <p className="editor-settings-group-label">Color scheme</p>
                    <div className="editor-settings-row">
                      <span className="editor-settings-row-label" id="editor-theme-select-label">
                        <SettingsLabelWithInfo
                          tipId="tip-theme-select"
                          tip="Choose light, dark, or follow your system appearance. Applies to the editor and voice overlay."
                        >
                          Theme
                        </SettingsLabelWithInfo>
                      </span>
                      <span className="editor-settings-row-control">
                        <EditorSelect
                          id="editor-theme-select"
                          labelledBy="editor-theme-select-label"
                          value={theme}
                          onChange={(v) => void saveTheme(normalizeThemePreference(v))}
                          options={[
                            { value: "system", label: "System" },
                            { value: "dark", label: "Dark" },
                            { value: "light", label: "Light" },
                          ]}
                        />
                      </span>
                    </div>
                  </section>

                  <section className="editor-settings-section editor-settings-section--compact">
                    <p className="editor-settings-group-label">Editor window</p>
                    <div className="editor-settings-row">
                      <div className="editor-settings-row-label">
                        <SettingsLabelWithInfo
                          tipId="tip-editor-transparency"
                          tip="How much desktop shows through the editor glass. Higher values increase transparency. Body text stays readable at the minimum opacity."
                        >
                          Transparency
                        </SettingsLabelWithInfo>
                      </div>
                      <div className="editor-settings-row-control editor-settings-row-control--slider">
                        <input
                          id="editor-window-transparency"
                          type="range"
                          min={0}
                          max={100}
                          step={1}
                          value={editorTransparency}
                          aria-valuenow={editorTransparency}
                          aria-valuemin={0}
                          aria-valuemax={100}
                          aria-valuetext={`${editorTransparency}%`}
                          aria-describedby="tip-editor-transparency"
                          onChange={(e) => {
                            const next = parseEditorTransparencySettingValue(e.target.value);
                            setEditorTransparency(next);
                            applyEditorTransparencyToDocument(next, theme);
                          }}
                          onPointerUp={(e) =>
                            void saveEditorTransparency(
                              parseEditorTransparencySettingValue(
                                (e.target as HTMLInputElement).value,
                              ),
                            )
                          }
                          onKeyUp={(e) => {
                            if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
                            void saveEditorTransparency(
                              parseEditorTransparencySettingValue(
                                (e.target as HTMLInputElement).value,
                              ),
                            );
                          }}
                        />
                        <span className="editor-settings-slider-value" aria-hidden>
                          {editorTransparency}%
                        </span>
                      </div>
                    </div>
                  </section>

                  <section className="editor-settings-section editor-settings-section--compact">
                    <p className="editor-settings-group-label">Voice overlay</p>
                    <div className="editor-settings-row">
                      <div className="editor-settings-row-label">
                        <SettingsLabelWithInfo
                          tipId="tip-hud-transparency"
                          tip="How much desktop shows through the HUD glass. Higher values increase transparency. Text stays readable at the minimum opacity."
                        >
                          Transparency
                        </SettingsLabelWithInfo>
                      </div>
                      <div className="editor-settings-row-control editor-settings-row-control--slider">
                        <input
                          id="editor-hud-transparency"
                          type="range"
                          min={0}
                          max={100}
                          step={1}
                          value={hudTransparency}
                          aria-valuenow={hudTransparency}
                          aria-valuemin={0}
                          aria-valuemax={100}
                          aria-valuetext={`${hudTransparency}%`}
                          aria-describedby="tip-hud-transparency"
                          onChange={(e) => {
                            const next = parseHudTransparencySettingValue(e.target.value);
                            setHudTransparency(next);
                            applyHudTransparencyToDocument(next, theme);
                          }}
                          onPointerUp={(e) =>
                            void saveHudTransparency(
                              parseHudTransparencySettingValue(
                                (e.target as HTMLInputElement).value,
                              ),
                            )
                          }
                          onKeyUp={(e) => {
                            if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
                            void saveHudTransparency(
                              parseHudTransparencySettingValue(
                                (e.target as HTMLInputElement).value,
                              ),
                            );
                          }}
                        />
                        <span className="editor-settings-slider-value" aria-hidden>
                          {hudTransparency}%
                        </span>
                      </div>
                    </div>
                  </section>
                </div>
              </div>
            )}

            {pane === "app-index" && <AppIndexPane onNotice={showSettingsNotice} />}
          </div>
        </div>
      )}

      {noticeText && (
        <p
          className={`editor-settings-notice editor-settings-notice--${noticePhase}`}
          role="status"
          aria-live="polite"
        >
          {noticeText}
        </p>
      )}
    </aside>
  );
}
