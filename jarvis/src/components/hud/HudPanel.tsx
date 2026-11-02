import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { emit, listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { useShallow } from "zustand/react/shallow";
import { useDebounced } from "../../hooks/useDebounced";
import { useHudStore } from "../../store/hudStore";
import {
  announcableText,
  selectCenterContent,
  type CenterSelectorInput,
  type CenterSelectorResult,
} from "./HudPanel.logic";
import { isHudOverlayShellActive } from "./hudOverlayPhases";
import { HUD_SHELL_TRANSITION_MS, hudShellEase } from "./hudMotion";
import { dictationMorphGeometry } from "./hudMorph";
import { HudText } from "./HudText";

/** Trailing debounce (ms) for streaming transcript in the SR-only live region. */
const TRANSCRIPT_ANNOUNCE_DEBOUNCE_MS = 380;

const HUD_SHELL_TRANSITION_S = HUD_SHELL_TRANSITION_MS / 1000;
const DECIDER_CONFIRM_MS = 6_000;
const DECIDER_SAVED_MS = 1_500;

type DeciderToolCall = { tool: string; args: Record<string, string> };
type DeciderConfirmPayload = { transcript: string; tool_calls: DeciderToolCall[] };
type DeciderConfirmState =
  | { id: number; kind: "confirm"; payload: DeciderConfirmPayload }
  | { id: number; kind: "saving" | "saved" | "error" };

/** Tauri drag script only checks `event.target`; delegate from child text/nodes up to the shell. */
function handleHudDragPointerDown(e: ReactPointerEvent<HTMLElement>) {
  if (e.button !== 0) return;
  const target = e.target as HTMLElement;
  if (!target.closest("[data-tauri-drag-region]")) return;
  if (target.closest("[data-tauri-drag-region]") === target) return;
  e.preventDefault();
  void getCurrentWindow().startDragging().catch(() => {});
}

/** Fixed sleeve height for bars — CSS `transform`/`opacity` (no layout thrash). */
const WAVE_BAR_SLEEVE_PX = 28;

function useHudCenterInput(): CenterSelectorInput {
  return useHudStore(
    useShallow((s) => ({
      phase: s.phase,
      transcript: s.transcript,
      match: s.match,
      actionText: s.actionText,
      actionError: s.actionError,
      audioError: s.audioError,
    })),
  );
}

const ORB_BARS = [0.45, 0.7, 0.9, 1, 0.9, 0.7, 0.45];

/**
 * Minimal dictation orb: bars that breathe with the mic, fixed in place. Drawn on a canvas in
 * a supersampled backing surface, with fractional bar heights for smooth motion at small sizes.
 */
function DictationOrb() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const reduceMotion = useReducedMotion();

  useEffect(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) return;
    let W = 0;
    let H = 0;
    let density = 0;
    const fit = () => {
      const dpr = Math.max(2, window.devicePixelRatio || 1);
      density = dpr;
      W = canvas.clientWidth;
      H = canvas.clientHeight;
      canvas.width = Math.round(W * dpr);
      canvas.height = Math.round(H * dpr);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    };
    const resize = new ResizeObserver(fit);
    resize.observe(canvas);
    fit();
    let color = getComputedStyle(canvas).color;
    const theme = new MutationObserver(() => { color = getComputedStyle(canvas).color; });
    theme.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });

    let level = 0;
    let last = performance.now();
    let raf = 0;
    const frame = (now: number) => {
      if (density !== Math.max(2, window.devicePixelRatio || 1)) fit();
      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      const { phase, amplitude } = useHudStore.getState();
      const raw = phase === "listening"
        ? Math.min(1, Math.pow(Math.max(0, amplitude - 0.02) * 3, 0.55))
        : 0;
      // Fast attack, slow release.
      level += (raw - level) * (1 - Math.exp(-(raw > level ? 28 : 7) * dt));
      const t = now / 1000;

      // Geometry stays in CSS pixels; extra backing pixels only improve antialiasing.
      const bw = Math.max(2, H / 8);
      const gap = Math.max(1, bw * 0.6);
      let n = ORB_BARS.length;
      while (n > 3 && n * bw + (n - 1) * gap > W * 0.72) n -= 2;
      const x0 = (W - (n * bw + (n - 1) * gap)) / 2;
      const maxH = H * 0.62;

      ctx.clearRect(0, 0, W, H);
      ctx.fillStyle = color;
      for (let i = 0; i < n; i++) {
        // Each bar moves on its own two sine voices, so the shape keeps changing, not just scaling.
        const k = i + (ORB_BARS.length - n) / 2;
        const wobble = reduceMotion ? 1 : 0.45 + 0.35 * Math.sin(t * (7 + k * 1.9) + k * 2.4)
          + 0.2 * Math.sin(t * (13 - k * 0.8) + k);
        // Fractional heights avoid visible one-pixel jumps on quiet speech.
        const h = bw + (maxH - bw) * Math.min(1, level * ORB_BARS[k] * wobble * 1.3);
        ctx.beginPath();
        ctx.roundRect(x0 + i * (bw + gap), (H - h) / 2, bw, h, bw / 2);
        ctx.fill();
      }
      raf = requestAnimationFrame(frame);
    };
    raf = requestAnimationFrame(frame);
    return () => {
      resize.disconnect();
      theme.disconnect();
      cancelAnimationFrame(raf);
    };
  }, [reduceMotion]);

  return <canvas ref={canvasRef} className="hud-orb" aria-hidden="true" />;
}

function WaveformBars() {
  const phase = useHudStore((s) => s.phase);
  const amplitude = useHudStore((s) => s.amplitude);
  const active = phase === "listening";
  const level = active ? amplitude : 0;

  const bars = useMemo(() => [0, 1, 2, 3, 4, 5, 6], []);

  return (
    <div
      className="hud-waveform"
      aria-hidden
    >
      {bars.map((i) => {
        const wave = 0.25 + 0.75 * Math.sin((i / 6) * Math.PI + level * 2.4);
        const rawH = 6 + level * wave * 22;
        const scaleY = active
          ? Math.max(0.14, Math.min(1, rawH / WAVE_BAR_SLEEVE_PX))
          : 0.14;

        return (
          <div
            key={i}
            className="hud-waveform-bar"
            style={{
              height: WAVE_BAR_SLEEVE_PX,
              transform: `scaleY(${scaleY})`,
              opacity: active ? 0.55 + level * 0.45 : 0.12,
            }}
          />
        );
      })}
    </div>
  );
}

function CenterContent({
  input,
  selected,
}: {
  input: CenterSelectorInput;
  selected: CenterSelectorResult;
}) {
  switch (selected.kind) {
    case "success":
      return (
        <svg className="hud-confirmation" viewBox="0 0 40 40" fill="none" aria-hidden="true">
          <circle cx="20" cy="20" r="17" stroke="currentColor" strokeWidth="1.5" opacity="0.3" />
          <path d="m12 20 5.5 5.5L28 15" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      );
    case "error":
      return <HudText text={selected.text} kind="error" />;
    case "match": {
      const m = input.match;
      if (!m) return <div className="hud-center-placeholder" aria-hidden />;
      return <HudText text={m.matched_phrase} kind="match" />;
    }
    case "action":
      return <HudText text={selected.text} kind="action" />;
    case "transcript":
      return <HudText text={selected.text} kind="transcript" />;
    default:
      return <div className="hud-center-placeholder" aria-hidden />;
  }
}

function HudShell({ centerInput }: { centerInput: CenterSelectorInput }) {
  const reduceMotion = useReducedMotion();
  const shellRef = useRef<HTMLDivElement>(null);
  const overlayMode = useHudStore((s) => s.overlayMode);
  const sessionId = useHudStore((s) => s.sessionId);
  const waitingForInput = useHudStore((s) => s.dictationWaiting);
  const isDictation = overlayMode === "dictation" && !waitingForInput;

  useLayoutEffect(() => {
    const shell = shellRef.current;
    const host = shell?.parentElement;
    if (!shell || !host) return;
    if (!isDictation) {
      shell.style.removeProperty("--hud-morph-inset");
      shell.style.removeProperty("--hud-morph-radius");
      return;
    }
    const sourceWidth = host.clientWidth;
    const sourceRadius = parseFloat(getComputedStyle(shell).borderRadius) || 12;
    const fit = () => {
      const { inset, radius } = dictationMorphGeometry(host.clientWidth, host.clientHeight, sourceWidth, sourceRadius);
      shell.style.setProperty("--hud-morph-inset", `${inset}px`);
      shell.style.setProperty("--hud-morph-radius", `${radius}px`);
    };
    fit();
    const observer = new ResizeObserver(fit);
    observer.observe(host);
    return () => observer.disconnect();
  }, [isDictation]);

  useEffect(() => {
    if (!isDictation || !/Windows/.test(navigator.userAgent)) return;
    let cancelled = false;
    let stop: (() => void) | undefined;
    let retry: ReturnType<typeof setInterval> | undefined;
    void listen<number>("hud-dictation-placed", ({ payload }) => {
      if (payload === sessionId) clearInterval(retry);
    }).then((unlisten) => {
      if (cancelled) { unlisten(); return; }
      stop = unlisten;
      // Native motion starts only after the initial shape has been committed.
      void emit("hud-dictation-ready", sessionId);
      retry = setInterval(() => { void emit("hud-dictation-ready", sessionId); }, 100);
    });
    return () => { cancelled = true; clearInterval(retry); stop?.(); };
  }, [isDictation, sessionId]);
  const selected = useMemo(
    () => selectCenterContent(centerInput),
    [centerInput],
  );
  const announceRaw = useMemo(
    () => announcableText(centerInput, selected),
    [centerInput, selected],
  );

  const srSource = selected.kind === "error" ? "" : announceRaw;
  const debounceMs =
    selected.kind === "transcript" ? TRANSCRIPT_ANNOUNCE_DEBOUNCE_MS : 0;
  const srAnnounced = useDebounced(srSource, debounceMs);

  const phase = centerInput.phase;

  // Always show mic chrome while `listening`. `match-result` can arrive before `hud-phase`
  // advances; hiding chrome when `match` is set produced an empty frosted shell with no level
  // indicator if the phase event was missed or reordered.
  const showListeningChrome = phase === "listening";

  /**
   * Backdrop-filter / inset shadows can outlast opacity on WebView2. Strip those layers as soon as
   * the session leaves overlay phases, so the rim fades with the rest of the shell.
   */
  useLayoutEffect(() => {
    const el = shellRef.current;
    if (!el) return;
    if (isHudOverlayShellActive(phase)) {
      for (const property of ["backdrop-filter", "-webkit-backdrop-filter", "box-shadow", "border"]) {
        el.style.removeProperty(property);
      }
      return;
    }
    el.style.setProperty("backdrop-filter", "none");
    el.style.setProperty("-webkit-backdrop-filter", "none");
    el.style.setProperty("box-shadow", "none");
    el.style.setProperty("border", "none");
  }, [phase]);

  const transition = useMemo(
    () =>
      reduceMotion
        ? { duration: 0.12, ease: "easeOut" as const }
        : { duration: HUD_SHELL_TRANSITION_S, ease: hudShellEase },
    [reduceMotion],
  );

  const shellInitial = { opacity: reduceMotion ? 1 : 0 };
  const shellExit = { opacity: 0 };

  return (
    <motion.div
      ref={shellRef}
      className={isDictation ? "hud-root hud-root--dictation" : "hud-root"}
      data-tauri-drag-region
      onPointerDown={handleHudDragPointerDown}
      role="region"
      aria-label={overlayMode === "dictation" ? "Dictation session" : "Voice session"}
      initial={shellInitial}
      animate={{ opacity: 1 }}
      exit={shellExit}
      transition={transition}
    >
      <AnimatePresence initial={false} mode="popLayout">
        {overlayMode === "dictation" && waitingForInput ? (
          <motion.div
            key="dictation-waiting"
            className="hud-body hud-body--solo"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: reduceMotion ? 0 : 0.1 }}
          >
            <div className="hud-phase-label">Dictation paused</div>
            <div className="hud-transcript-wrap" role="status" aria-live="polite">
              <div className="hud-transcript-visual">
                <HudText text="Click a text field to start dictating." kind="transcript" />
              </div>
            </div>
            <div className="hud-phase-label">Microphone paused</div>
          </motion.div>
        ) : isDictation ? (
          <motion.div
            key="dictation"
            className="hud-body hud-body--dictation"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: reduceMotion ? 0 : 0.14, delay: reduceMotion ? 0 : 0.12 }}
          >
            <DictationOrb />
          </motion.div>
        ) : (
          <motion.div
            key="command"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: reduceMotion ? 0 : 0.1 }}
            className={showListeningChrome ? "hud-body" : "hud-body hud-body--solo"}
          >
            <div className="hud-transcript-wrap">
              <span className="hud-sr-only" role="status" aria-live="polite" aria-atomic="true">
                {srAnnounced}
              </span>
              <div className="hud-transcript-visual" aria-hidden={selected.kind !== "error"}>
                <CenterContent input={centerInput} selected={selected} />
              </div>
            </div>
            {showListeningChrome && (
              <div className="hud-bottom-bar">
                <WaveformBars />
              </div>
            )}
          </motion.div>
        )}
      </AnimatePresence>
    </motion.div>
  );
}

/** Fade / pulse wrapper: only mount animated shell while HUD session is active. */
function HudBody() {
  const centerInput = useHudCenterInput();
  const active = isHudOverlayShellActive(centerInput.phase);

  // Native hide waits for this exit; command-to-dictation keeps the same visible shell.
  return (
    <AnimatePresence>
      {active ? <HudShell key="shell" centerInput={centerInput} /> : null}
    </AnimatePresence>
  );
}

export function HudPanel() {
  const [deciderConfirm, setDeciderConfirm] = useState<DeciderConfirmState | null>(null);
  const confirmIdRef = useRef(0);
  const phaseRef = useRef({ phase: "idle", sessionId: 0 });
  const activeConfirmationRef = useRef(false);
  const activeConfirmationSessionRef = useRef<number | null>(null);

  useEffect(() => {
    let unlistenConfirm: (() => void) | undefined;
    let unlistenPhase: (() => void) | undefined;
    let cancelled = false;
    const showConfirmation = (
      payload: DeciderConfirmPayload,
      sessionId: number,
    ) => {
      const id = ++confirmIdRef.current;
      activeConfirmationRef.current = true;
      activeConfirmationSessionRef.current = sessionId;
      setDeciderConfirm({ id, kind: "confirm", payload });
      // Done is click-through by default; make the prompt clickable without focusing the app.
      void getCurrentWindow().show()
        .then(() => {
          if (
            confirmIdRef.current === id &&
            phaseRef.current.phase === "done" &&
            phaseRef.current.sessionId === sessionId
          ) {
            return getCurrentWindow().setIgnoreCursorEvents(false);
          }
        })
        .catch(() => {});
    };
    void Promise.all([
      listen<DeciderConfirmPayload>("decider-confirm", (event) => {
        const state = useHudStore.getState();
        const sessionId = phaseRef.current.sessionId;
        if (
          phaseRef.current.phase !== "done" ||
          state.sessionId !== sessionId
        ) {
          return;
        }
        showConfirmation(event.payload, sessionId);
      }),
      listen<{ phase: string; session_id?: number }>("hud-phase", (event) => {
        const state = useHudStore.getState();
        const sessionId = event.payload.session_id ?? state.sessionId;
        if (sessionId < phaseRef.current.sessionId) return;
        phaseRef.current = { phase: event.payload.phase, sessionId };
        if (event.payload.phase === "done") {
          if (
            activeConfirmationRef.current &&
            activeConfirmationSessionRef.current === sessionId
          ) {
            void getCurrentWindow().setIgnoreCursorEvents(false).catch(() => {});
          }
          return;
        }
        activeConfirmationRef.current = false;
        activeConfirmationSessionRef.current = null;
        confirmIdRef.current += 1;
        setDeciderConfirm(null);
      }),
    ]).then(([stopConfirm, stopPhase]) => {
      if (cancelled) {
        stopConfirm();
        stopPhase();
      } else {
        unlistenConfirm = stopConfirm;
        unlistenPhase = stopPhase;
      }
    });
    return () => {
      cancelled = true;
      unlistenConfirm?.();
      unlistenPhase?.();
    };
  }, []);

  useEffect(() => {
    if (!deciderConfirm) return;
    const timeout = window.setTimeout(
      () => {
        if (confirmIdRef.current !== deciderConfirm.id) return;
        const sessionId = activeConfirmationSessionRef.current;
        activeConfirmationRef.current = false;
        activeConfirmationSessionRef.current = null;
        confirmIdRef.current += 1;
        setDeciderConfirm(null);
        if (
          phaseRef.current.phase === "done" &&
          phaseRef.current.sessionId === sessionId
        ) {
          void getCurrentWindow().setIgnoreCursorEvents(true).catch(() => {});
        }
      },
      deciderConfirm.kind === "confirm" || deciderConfirm.kind === "saving"
        ? DECIDER_CONFIRM_MS
        : DECIDER_SAVED_MS,
    );
    return () => window.clearTimeout(timeout);
  }, [deciderConfirm]);

  const saveDeciderCommand = async (id: number, payload: DeciderConfirmPayload) => {
    if (confirmIdRef.current !== id) return;
    setDeciderConfirm({ id, kind: "saving" });
    void getCurrentWindow().setIgnoreCursorEvents(true).catch(() => {});
    try {
      await invoke("save_learned_command", {
        transcript: payload.transcript,
        toolCalls: payload.tool_calls,
      });
      if (
        confirmIdRef.current === id &&
        phaseRef.current.phase === "done" &&
        phaseRef.current.sessionId === activeConfirmationSessionRef.current
      ) {
        setDeciderConfirm({ id, kind: "saved" });
      }
    } catch (error) {
      console.error("save_learned_command failed", error);
      if (
        confirmIdRef.current === id &&
        phaseRef.current.phase === "done" &&
        phaseRef.current.sessionId === activeConfirmationSessionRef.current
      ) {
        setDeciderConfirm({ id, kind: "error" });
      }
    }
  };

  return (
    <div
      className="hud-panel-fill"
      data-tauri-drag-region
      onPointerDown={handleHudDragPointerDown}
    >
      {deciderConfirm && (
        <div
          role="dialog"
          aria-label={deciderConfirm.kind === "confirm" ? "Save learned command" : "Command status"}
          onPointerDown={(event) => event.stopPropagation()}
          style={{
            position: "fixed",
            inset: 0,
            zIndex: 10,
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            gap: 12,
            padding: 12,
            background: "rgba(20, 20, 24, 0.96)",
            color: "white",
            font: "500 14px system-ui, sans-serif",
          }}
        >
          {deciderConfirm.kind === "confirm" ? (
            <>
              <span>Did that work?</span>
              <button
                type="button"
                onClick={() => void saveDeciderCommand(deciderConfirm.id, deciderConfirm.payload)}
              >
                Yes
              </button>
              <button
                type="button"
                onClick={() => {
                  const sessionId = activeConfirmationSessionRef.current;
                  if (confirmIdRef.current === deciderConfirm.id) {
                    activeConfirmationRef.current = false;
                    activeConfirmationSessionRef.current = null;
                    confirmIdRef.current += 1;
                  }
                  setDeciderConfirm(null);
                  if (
                    phaseRef.current.phase === "done" &&
                    phaseRef.current.sessionId === sessionId
                  ) {
                    void getCurrentWindow().setIgnoreCursorEvents(true).catch(() => {});
                  }
                }}
              >
                No
              </button>
            </>
          ) : (
            <span role="status">
              {deciderConfirm.kind === "saving"
                ? "Saving…"
                : deciderConfirm.kind === "saved"
                  ? "Saved"
                  : "Couldn’t save"}
            </span>
          )}
        </div>
      )}
      <HudBody />
    </div>
  );
}
