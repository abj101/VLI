import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { KeyboardEvent as ReactKeyboardEvent, RefObject } from "react";
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import {
  applyEditorThemeToDocument,
  applyEditorTransparencyToDocument,
  applyHudTransparencyToDocument,
  normalizeSttProvider,
  normalizeThemePreference,
  parseEditorTransparencySettingValue,
  parseHudTransparencySettingValue,
  parseRemoteSttTimeoutSecs,
  EDITOR_TRANSPARENCY_DEFAULT,
  HUD_TRANSPARENCY_DEFAULT,
  parseThresholdSettingValue,
  shouldWarmupWhisperGpu,
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
  type SttProvider,
} from "../editor/SettingsPanel.logic";
import { formatUserError } from "../../utils/userErrors";
import { EDITOR_SETTINGS_NAV, type EditorSettingsNavId } from "./settingsNav";
import { AppIndexPane } from "./AppIndexPane";
import { HotkeyChordDisplay } from "./HotkeyChordDisplay";
import { SettingsLabelWithInfo } from "./SettingsInfoTip";
import { EditorSelect } from "../ui/EditorSelect";

const HOTKEY_KEY = "hotkey";
const THEME_KEY = "theme";
const HUD_TRANSPARENCY_KEY = "hud_transparency";
const EDITOR_TRANSPARENCY_KEY = "editor_transparency";
const DEFAULT_THRESHOLD_KEY = "default_fuzzy_threshold_pct";
export type { EditorSettingsNavId } from "./settingsNav";
export { EDITOR_SETTINGS_NAV } from "./settingsNav";

type AppSettingsPayload = {
  wakeEngine: string;
  owwThreshold: number;
  sttProvider: string;
  remoteSttUrl: string;
  remoteSttModel: string | null;
  remoteSttTimeoutSecs: number;
  remoteSttKeyStored: boolean;
  localWhisperUseGpu: boolean;
  llmRouterModelPath: string | null;
  llmRouterConfidenceThreshold: number;
  llmRouterTier2Enabled: boolean;
};

type RouterStatusPayload = {
  featureCompiled: boolean;
  compileBackend: "none" | "vulkan" | "cuda" | "metal" | string;
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

type WhisperGpuStatusPayload = {
  compileBackend: "none" | "vulkan" | "cuda" | "metal" | string;
  runtimeAvailable: boolean;
  message: string | null;
};

type WhisperGpuWarmupPayload = {
  ready: boolean;
  message: string;
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
  onClose?: () => void;
  returnFocusRef?: RefObject<HTMLElement | null>;
  /** When set, panel is embedded in the editor main column (no overlay chrome). */
  embedded?: boolean;
  /** Which pane to show when `embedded` is true (controlled by parent sidebar). */
  activeNav?: EditorSettingsNavId;
};

export function SettingsPanel({
  onClose,
  returnFocusRef,
  embedded = false,
  activeNav,
}: SettingsPanelProps) {
  const [loading, setLoading] = useState(true);
  const [hotkey, setHotkey] = useState("ctrl+shift+j");
  const [threshold, setThreshold] = useState(0.8);
  const [theme, setTheme] = useState<EditorThemePreference>("system");
  const [hudTransparency, setHudTransparency] = useState(HUD_TRANSPARENCY_DEFAULT);
  const [editorTransparency, setEditorTransparency] = useState(EDITOR_TRANSPARENCY_DEFAULT);
  const [hotkeyError, setHotkeyError] = useState<string | null>(null);
  const [savingHotkey, setSavingHotkey] = useState(false);
  const [hotkeyRecording, setHotkeyRecording] = useState(false);
  const [hotkeyCapturedThisSession, setHotkeyCapturedThisSession] = useState(false);

  const [wakeEngine, setWakeEngine] = useState("oww");
  const [owwThreshold, setOwwThreshold] = useState(0.7);

  const [sttProvider, setSttProvider] = useState<SttProvider>("local");
  const [remoteSttUrl, setRemoteSttUrl] = useState("");
  const [remoteSttModel, setRemoteSttModel] = useState("");
  const [remoteSttTimeoutSecs, setRemoteSttTimeoutSecs] = useState(30);
  const [remoteSttKeyStored, setRemoteSttKeyStored] = useState(false);
  const [remoteSttKeyInput, setRemoteSttKeyInput] = useState("");
  const [savingRemoteStt, setSavingRemoteStt] = useState(false);
  const [localWhisperUseGpu, setLocalWhisperUseGpu] = useState(false);
  const [whisperGpuPreparing, setWhisperGpuPreparing] = useState(false);
  const [whisperGpuPrepMessage, setWhisperGpuPrepMessage] = useState<string | null>(null);
  const [whisperGpuStatus, setWhisperGpuStatus] = useState<WhisperGpuStatusPayload>({
    compileBackend: "none",
    runtimeAvailable: false,
    message: "This build was compiled without a Whisper GPU backend.",
  });
  const [llmRouterTier2Enabled, setLlmRouterTier2Enabled] = useState(false);
  const [llmRouterConfidenceThreshold, setLlmRouterConfidenceThreshold] = useState(0.7);
  const [llmRouterModelPath, setLlmRouterModelPath] = useState("");
  const [routerPreparing, setRouterPreparing] = useState(false);
  const [routerPrepMessage, setRouterPrepMessage] = useState<string | null>(null);
  const [routerStatus, setRouterStatus] = useState<RouterStatusPayload>({
    featureCompiled: false,
    compileBackend: "none",
    runtimeAvailable: false,
    modelPresent: false,
    modelPath: null,
    tier2Enabled: false,
    confidenceThreshold: 0.7,
    message: "LLM router is not available in this build.",
  });

  const panelRef = useRef<HTMLElement | null>(null);
  const hotkeyRecordingStateRef = useRef<HotkeyRecordingState>(createHotkeyRecordingState());
  const hotkeyBeforeRecordingRef = useRef<string | null>(null);
  const hotkeysNavRef = useRef<HTMLButtonElement>(null);
  const [internalNav, setInternalNav] = useState<EditorSettingsNavId>("hotkeys");
  const pane = embedded && activeNav != null ? activeNav : internalNav;

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

  useEffect(() => {
    if (embedded && activeNav) {
      setInternalNav(activeNav);
    }
  }, [embedded, activeNav]);

  const refreshFromBackend = async () => {
    const [
      savedHotkey,
      savedThreshold,
      savedTheme,
      savedHudTransparency,
      savedEditorTransparency,
      app,
      gpuStatus,
      router,
    ] = await Promise.all([
        invoke<string | null>("get_setting", { key: HOTKEY_KEY }),
        invoke<string | null>("get_setting", { key: DEFAULT_THRESHOLD_KEY }),
        invoke<string | null>("get_setting", { key: THEME_KEY }),
        invoke<string | null>("get_setting", { key: HUD_TRANSPARENCY_KEY }),
        invoke<string | null>("get_setting", { key: EDITOR_TRANSPARENCY_KEY }),
        invoke<AppSettingsPayload>("get_settings"),
        invoke<WhisperGpuStatusPayload>("whisper_gpu_status"),
        invoke<RouterStatusPayload>("router_status"),
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
    const parsedHudTransparency = parseHudTransparencySettingValue(savedHudTransparency);
    setHudTransparency(parsedHudTransparency);
    applyHudTransparencyToDocument(parsedHudTransparency, normalizedTheme);
    const parsedEditorTransparency = parseEditorTransparencySettingValue(savedEditorTransparency);
    setEditorTransparency(parsedEditorTransparency);
    applyEditorTransparencyToDocument(parsedEditorTransparency, normalizedTheme);
    setWakeEngine(app.wakeEngine);
    setOwwThreshold(app.owwThreshold);
    setSttProvider(normalizeSttProvider(app.sttProvider));
    setRemoteSttUrl(app.remoteSttUrl ?? "");
    setRemoteSttModel(app.remoteSttModel ?? "");
    setRemoteSttTimeoutSecs(app.remoteSttTimeoutSecs);
    setRemoteSttKeyStored(app.remoteSttKeyStored);
    setLocalWhisperUseGpu(app.localWhisperUseGpu);
    setWhisperGpuStatus(gpuStatus);
    setLlmRouterTier2Enabled(app.llmRouterTier2Enabled);
    setLlmRouterConfidenceThreshold(app.llmRouterConfidenceThreshold);
    setLlmRouterModelPath(app.llmRouterModelPath ?? "");
    setRouterStatus(router);
  };

  const whisperGpuCanEnable =
    whisperGpuStatus.compileBackend !== "none" && whisperGpuStatus.runtimeAvailable;

  const routerCanEnable =
    routerStatus.featureCompiled && routerStatus.modelPresent;

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

  useLayoutEffect(() => {
    if (loading || embedded) return;
    hotkeysNavRef.current?.focus();
  }, [loading, embedded]);

  useEffect(() => {
    if (!onClose || embedded) return;
    const onKey = (ev: Event) => {
      const e = ev as KeyboardEvent;
      if (hotkeyRecording) return;
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, embedded, hotkeyRecording]);

  useEffect(() => {
    void invoke("set_hotkey_recording", { recording: hotkeyRecording });
    return () => {
      void invoke("set_hotkey_recording", { recording: false });
    };
  }, [hotkeyRecording]);

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

  useEffect(() => {
    const focusTarget = returnFocusRef?.current ?? null;
    return () => {
      focusTarget?.focus();
    };
  }, [returnFocusRef]);

  const onPanelKeyDown = (e: ReactKeyboardEvent<HTMLElement>) => {
    if (embedded || e.key !== "Tab" || !panelRef.current) return;
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
    const clamped = Math.max(0.5, Math.min(1, nextThreshold));
    if (clamped !== threshold) {
      setThreshold(clamped);
    }
    const pct = Math.round(clamped * 100);
    try {
      await invoke("set_setting", {
        key: DEFAULT_THRESHOLD_KEY,
        value: String(pct),
      });
    } catch (err) {
      showSettingsNotice(formatUserError(err, "Could not save the default match threshold."));
    }
  };

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
      showSettingsNotice(formatUserError(err, "Could not save the transcription provider."));
    }
  };

  const persistLlmRouterTier2 = async (next: boolean) => {
    const prev = llmRouterTier2Enabled;
    setLlmRouterTier2Enabled(next);
    if (!next) {
      setRouterPreparing(false);
      setRouterPrepMessage(null);
    } else if (routerCanEnable) {
      setRouterPreparing(true);
      setRouterPrepMessage("Warming router model…");
    }
    try {
      const s = await invoke<AppSettingsPayload>("update_settings", {
        patch: { llmRouterTier2Enabled: next },
      });
      setLlmRouterTier2Enabled(s.llmRouterTier2Enabled);
      if (next && s.llmRouterTier2Enabled && routerCanEnable) {
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

  const commitLlmRouterConfidence = async (next: number) => {
    const clamped = Math.max(0, Math.min(1, next));
    setLlmRouterConfidenceThreshold(clamped);
    try {
      await invoke<AppSettingsPayload>("update_settings", {
        patch: { llmRouterConfidenceThreshold: clamped },
      });
    } catch (err) {
      showSettingsNotice(formatUserError(err, "Could not save router confidence threshold."));
    }
  };

  const saveLlmRouterModelPath = async () => {
    try {
      const s = await invoke<AppSettingsPayload>("update_settings", {
        patch: { llmRouterModelPath: llmRouterModelPath.trim() || "" },
      });
      setLlmRouterModelPath(s.llmRouterModelPath ?? "");
      const status = await invoke<RouterStatusPayload>("router_status");
      setRouterStatus(status);
      showSettingsNotice("Router model path saved");
    } catch (err) {
      showSettingsNotice(formatUserError(err, "Could not save router model path."));
    }
  };

  const persistLocalWhisperUseGpu = async (next: boolean) => {
    const prev = localWhisperUseGpu;
    setLocalWhisperUseGpu(next);
    const shouldWarmup = shouldWarmupWhisperGpu({
      nextEnabled: next,
      compileBackend: whisperGpuStatus.compileBackend,
      runtimeAvailable: whisperGpuStatus.runtimeAvailable,
    });
    if (!next) {
      setWhisperGpuPreparing(false);
      setWhisperGpuPrepMessage(null);
    } else if (shouldWarmup) {
      setWhisperGpuPreparing(true);
      setWhisperGpuPrepMessage("Preparing Vulkan model...");
    } else {
      setWhisperGpuPrepMessage(null);
    }
    try {
      const s = await invoke<AppSettingsPayload>("update_settings", {
        patch: { localWhisperUseGpu: next },
      });
      setLocalWhisperUseGpu(s.localWhisperUseGpu);
      if (shouldWarmup && s.localWhisperUseGpu) {
        const unlisten = await listen<WhisperGpuWarmupPayload>("whisper-gpu-warmup", (event) => {
          setWhisperGpuPrepMessage(event.payload.message);
          showSettingsNotice(event.payload.message);
          setWhisperGpuPreparing(false);
          void unlisten();
        });
        const warmup = await invoke<WhisperGpuWarmupPayload>("warmup_whisper_gpu");
        setWhisperGpuPrepMessage(warmup.message);
        if (warmup.ready) {
          showSettingsNotice(warmup.message);
          setWhisperGpuPreparing(false);
          void unlisten();
        }
      } else {
        showSettingsNotice(
          next ? "Whisper will use GPU on next listen." : "Whisper will use CPU on next listen.",
        );
      }
    } catch (err) {
      setLocalWhisperUseGpu(prev);
      showSettingsNotice(formatUserError(err, "Could not save the Whisper GPU option."));
      setWhisperGpuPrepMessage(null);
    } finally {
      if (shouldWarmup) {
        setWhisperGpuPreparing(false);
      }
    }
  };

  const saveRemoteSttEndpoint = async () => {
    if (sttProvider !== "remote") return;
    const timeout = parseRemoteSttTimeoutSecs(String(remoteSttTimeoutSecs));
    if (timeout === null) {
      showSettingsNotice("Remote STT timeout must be between 1 and 300 seconds.");
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
      showSettingsNotice("Remote STT settings saved");
    } catch (err) {
      showSettingsNotice(formatUserError(err, "Could not save remote speech settings."));
    } finally {
      setSavingRemoteStt(false);
    }
  };

  const saveRemoteSttKey = async () => {
    if (!remoteSttKeyInput.trim()) {
      showSettingsNotice("Enter an API key before saving.");
      return;
    }
    setSavingRemoteStt(true);
    try {
      await invoke("save_api_key", { service: "remote_stt", key: remoteSttKeyInput });
      setRemoteSttKeyInput("");
      await refreshFromBackend();
      showSettingsNotice("Remote STT API key saved to OS keychain");
    } catch (err) {
      showSettingsNotice(formatUserError(err, "Could not save the remote speech API key."));
    } finally {
      setSavingRemoteStt(false);
    }
  };

  const clearRemoteSttKey = async () => {
    setSavingRemoteStt(true);
    try {
      await invoke("delete_api_key", { service: "remote_stt" });
      await refreshFromBackend();
      showSettingsNotice("Remote STT key cleared");
    } catch (err) {
      showSettingsNotice(formatUserError(err, "Could not clear the remote speech API key."));
    } finally {
      setSavingRemoteStt(false);
    }
  };

  return (
    <aside
      ref={panelRef}
      className={embedded ? "editor-settings-embedded" : "editor-settings-panel"}
      role={embedded ? undefined : "dialog"}
      aria-modal={embedded ? undefined : true}
      aria-busy={loading}
      aria-label={embedded ? undefined : "Settings"}
      onKeyDown={onPanelKeyDown}
    >
      {!embedded && (
        <header className="editor-settings-header">
          <h2 id="settings-dialog-title">Settings</h2>
          {onClose && (
            <button type="button" className="editor-btn" onClick={onClose} aria-label="Close settings">
              Close
            </button>
          )}
        </header>
      )}
      {loading ? (
        <p className="editor-settings-loading" aria-live="polite">
          Loading settings…
        </p>
      ) : (
        <div className="editor-settings-body">
          {!embedded && (
            <nav className="editor-settings-nav" aria-label="Settings categories">
              {EDITOR_SETTINGS_NAV.map((item) => (
                <button
                  key={item.id}
                  id={`editor-settings-nav-${item.id}`}
                  ref={item.id === "hotkeys" ? hotkeysNavRef : undefined}
                  type="button"
                  className={`editor-settings-nav-btn${internalNav === item.id ? " is-active" : ""}`}
                  aria-current={internalNav === item.id ? "page" : undefined}
                  onClick={() => setInternalNav(item.id)}
                >
                  {item.label}
                </button>
              ))}
            </nav>
          )}
          <div className={`editor-settings-pane${embedded ? " editor-settings-pane--solo" : ""}`}>
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
              </div>
            )}

            {pane === "recognition" && (
              <div
                className="editor-settings-content editor-settings-content--recognition"
                aria-labelledby={`editor-settings-nav-${pane}`}
              >
                <div className="editor-settings-recognition-form">
                  <section className="editor-settings-section editor-settings-section--compact">
                    <p className="editor-settings-group-label">Command matching</p>
                    <div className="editor-settings-row">
                      <div className="editor-settings-row-label">
                        <SettingsLabelWithInfo
                          tipId="tip-default-threshold"
                          tip="How closely speech must match command triggers. Higher is stricter. Used as the default for new commands."
                        >
                          Match Threshold
                        </SettingsLabelWithInfo>
                      </div>
                      <div className="editor-settings-row-control editor-settings-row-control--slider">
                        <input
                          type="range"
                          min={0}
                          max={1}
                          step={0.01}
                          value={threshold}
                          aria-valuenow={threshold}
                          aria-valuemin={0}
                          aria-valuemax={1}
                          aria-valuetext={threshold.toFixed(2)}
                          aria-describedby="tip-default-threshold"
                          onChange={(e) => setThreshold(Number(e.target.value))}
                          onPointerUp={(e) =>
                            void saveThreshold(Number((e.target as HTMLInputElement).value))
                          }
                          onKeyUp={(e) => {
                            if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
                            void saveThreshold(Number((e.target as HTMLInputElement).value));
                          }}
                        />
                        <span className="editor-settings-slider-value" aria-hidden>
                          {threshold.toFixed(2)}
                        </span>
                      </div>
                    </div>
                  </section>

                  <section className="editor-settings-section editor-settings-section--compact">
                    <p className="editor-settings-group-label">Transcription</p>
                    <div className="editor-settings-row">
                      <span className="editor-settings-row-label" id="editor-stt-provider-label">
                        <SettingsLabelWithInfo
                          tipId="tip-stt-provider"
                          tip="Converts speech to text for command matching. Remote mode needs HTTPS and an API key stored in the OS keychain."
                        >
                          Provider
                        </SettingsLabelWithInfo>
                      </span>
                      <span className="editor-settings-row-control">
                        <EditorSelect
                          id="editor-stt-provider"
                          labelledBy="editor-stt-provider-label"
                          value={sttProvider}
                          onChange={(v) => void persistSttProvider(normalizeSttProvider(v))}
                          options={[
                            { value: "local", label: "On-device (Whisper)" },
                            { value: "remote", label: "Remote HTTP API" },
                          ]}
                        />
                      </span>
                    </div>

                    {sttProvider === "local" && (
                      <div className="editor-settings-row editor-settings-row--switch">
                        <div className="editor-settings-row-label editor-settings-row-label--stack">
                          <SettingsLabelWithInfo
                            id="editor-whisper-gpu-label"
                            tipId="tip-whisper-gpu"
                            tip="Runs Whisper on your GPU when this build supports it. Uses CPU if unavailable or turned off."
                          >
                            GPU acceleration
                          </SettingsLabelWithInfo>
                          {(whisperGpuPreparing ||
                            whisperGpuPrepMessage ||
                            !whisperGpuCanEnable) && (
                            <span
                              className="editor-settings-switch-meta"
                              role="status"
                              aria-live="polite"
                            >
                              {whisperGpuPreparing && (
                                <span className="editor-settings-spinner" aria-hidden />
                              )}
                              {whisperGpuPreparing
                                ? (whisperGpuPrepMessage ?? "Preparing model…")
                                : (whisperGpuPrepMessage ??
                                  whisperGpuStatus.message ??
                                  (!whisperGpuCanEnable
                                    ? "No GPU backend in this build"
                                    : null))}
                            </span>
                          )}
                        </div>
                        <div className="editor-settings-row-control editor-settings-row-control--switch">
                          <button
                            type="button"
                            id="editor-whisper-gpu"
                            className={`editor-switch${localWhisperUseGpu ? " is-on" : ""}`}
                            role="switch"
                            aria-labelledby="editor-whisper-gpu-label"
                            aria-checked={localWhisperUseGpu}
                            disabled={!whisperGpuCanEnable}
                            onClick={() => void persistLocalWhisperUseGpu(!localWhisperUseGpu)}
                          >
                            <span className="editor-switch-knob" />
                          </button>
                        </div>
                      </div>
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
                          className="editor-btn editor-btn--primary"
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
                      <div className="editor-settings-inline--actions">
                        <button
                          type="button"
                          className="editor-btn editor-btn--primary"
                          onClick={() => void saveRemoteSttKey()}
                          disabled={savingRemoteStt}
                        >
                          {savingRemoteStt ? "Saving…" : "Save"}
                        </button>
                        <button
                          type="button"
                          className="editor-btn"
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

                  <section className="editor-settings-section editor-settings-section--compact">
                    <p className="editor-settings-group-label">LLM router (Tier 2)</p>
                    <div className="editor-settings-row editor-settings-row--switch">
                      <div className="editor-settings-row-label editor-settings-row-label--stack">
                        <SettingsLabelWithInfo
                          id="editor-llm-router-tier2-label"
                          tipId="tip-llm-router-tier2"
                          tip="Routes natural speech to tools with a small on-device model. Requires the router GGUF and an llm-local build."
                        >
                          Enable Tier 2 routing
                        </SettingsLabelWithInfo>
                        {(routerPreparing ||
                          routerPrepMessage ||
                          !routerCanEnable ||
                          routerStatus.message) && (
                          <span
                            className="editor-settings-switch-meta"
                            role="status"
                            aria-live="polite"
                          >
                            {routerPreparing && (
                              <span className="editor-settings-spinner" aria-hidden />
                            )}
                            {routerPreparing
                              ? (routerPrepMessage ?? "Preparing model…")
                              : (routerPrepMessage ??
                                routerStatus.message ??
                                (!routerCanEnable
                                  ? "Router model or feature unavailable"
                                  : null))}
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
                          disabled={!routerCanEnable}
                          onClick={() => void persistLlmRouterTier2(!llmRouterTier2Enabled)}
                        >
                          <span className="editor-switch-knob" />
                        </button>
                      </div>
                    </div>
                    <div className="editor-settings-row">
                      <div className="editor-settings-row-label">
                        <SettingsLabelWithInfo
                          tipId="tip-llm-router-confidence"
                          tip="Reject tool calls when the model confidence is below this value."
                        >
                          Confidence threshold
                        </SettingsLabelWithInfo>
                      </div>
                      <div className="editor-settings-row-control editor-settings-row-control--slider">
                        <input
                          type="range"
                          min={0}
                          max={1}
                          step={0.01}
                          value={llmRouterConfidenceThreshold}
                          aria-valuenow={llmRouterConfidenceThreshold}
                          aria-valuemin={0}
                          aria-valuemax={1}
                          aria-valuetext={llmRouterConfidenceThreshold.toFixed(2)}
                          aria-describedby="tip-llm-router-confidence"
                          onChange={(e) =>
                            setLlmRouterConfidenceThreshold(Number(e.target.value))
                          }
                          onPointerUp={(e) =>
                            void commitLlmRouterConfidence(
                              Number((e.target as HTMLInputElement).value),
                            )
                          }
                          onKeyUp={(e) => {
                            if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
                            void commitLlmRouterConfidence(
                              Number((e.target as HTMLInputElement).value),
                            );
                          }}
                        />
                        <span className="editor-settings-slider-value" aria-hidden>
                          {llmRouterConfidenceThreshold.toFixed(2)}
                        </span>
                      </div>
                    </div>
                    <label htmlFor="editor-llm-router-model-path">
                      Model path (optional)
                      <input
                        id="editor-llm-router-model-path"
                        type="text"
                        autoComplete="off"
                        placeholder="Default: bundled qwen2.5 router GGUF"
                        value={llmRouterModelPath}
                        onChange={(e) => setLlmRouterModelPath(e.target.value)}
                      />
                    </label>
                    <div className="editor-settings-inline">
                      <button
                        type="button"
                        className="editor-btn editor-btn--primary"
                        onClick={() => void saveLlmRouterModelPath()}
                      >
                        Save model path
                      </button>
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
