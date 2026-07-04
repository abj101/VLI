import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { useLayoutEffect, useMemo, useRef, type PointerEvent as ReactPointerEvent } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { HudOverlayMode } from "../../types";
import { useShallow } from "zustand/react/shallow";
import { useDebounced } from "../../hooks/useDebounced";
import { useHudStore } from "../../store/hudStore";
import {
  announcableText,
  selectCenterContent,
  selectPhaseLabel,
  type CenterSelectorInput,
  type CenterSelectorResult,
} from "./HudPanel.logic";
import { isHudOverlayShellActive } from "./hudOverlayPhases";
import { HUD_SHELL_TRANSITION_MS, hudShellEase } from "./hudMotion";

/** Trailing debounce (ms) for streaming transcript in the SR-only live region. */
const TRANSCRIPT_ANNOUNCE_DEBOUNCE_MS = 380;

const HUD_SHELL_TRANSITION_S = HUD_SHELL_TRANSITION_MS / 1000;

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

function WaveformBars({ dictation = false }: { dictation?: boolean }) {
  const phase = useHudStore((s) => s.phase);
  const amplitude = useHudStore((s) => s.amplitude);
  const active = phase === "listening";
  const level = active ? amplitude : 0;
  // Dictation: boost mic level so bars read taller in the compact HUD.
  const visualLevel = dictation && active ? Math.min(1, level * 1.35 + 0.08) : level;

  const bars = useMemo(() => [0, 1, 2, 3, 4, 5, 6], []);

  return (
    <div
      className={dictation ? "hud-waveform hud-waveform--dictation" : "hud-waveform"}
      aria-hidden
    >
      {bars.map((i) => {
        const wave = 0.25 + 0.75 * Math.sin((i / 6) * Math.PI + visualLevel * 2.4);
        let scaleY: number;
        if (dictation) {
          scaleY = active
            ? Math.max(0.22, Math.min(1, 0.18 + visualLevel * wave * 0.82))
            : 0.22;
        } else {
          const rawH = 6 + level * wave * 22;
          scaleY = active
            ? Math.max(0.14, Math.min(1, rawH / WAVE_BAR_SLEEVE_PX))
            : 0.14;
        }

        return (
          <div
            key={i}
            className="hud-waveform-bar"
            style={{
              ...(dictation ? {} : { height: WAVE_BAR_SLEEVE_PX }),
              transform: `scaleY(${scaleY})`,
              opacity: active ? 0.55 + visualLevel * 0.45 : 0.12,
            }}
          />
        );
      })}
    </div>
  );
}

function RecognizedPhrase({ phrase, motionKey }: { phrase: string; motionKey: string }) {
  const reduceMotion = useReducedMotion();

  return (
    <div
      key={motionKey}
      className={
        reduceMotion
          ? "hud-recognized-wrap"
          : "hud-recognized-wrap hud-recognized-wrap--enter"
      }
    >
      <span className="hud-recognized-text">{phrase}</span>
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
    case "error":
      return (
        <div className="hud-line hud-line-error" role="alert">
          {selected.text}
        </div>
      );
    case "match": {
      const m = input.match;
      if (!m) return <div className="hud-center-placeholder" aria-hidden />;
      const motionKey = `${m.node_id}-${m.matched_phrase}`;
      return (
        <RecognizedPhrase
          phrase={m.matched_phrase}
          motionKey={motionKey}
        />
      );
    }
    case "action":
      return (
        <div className="hud-line hud-line-action">
          <span>{selected.text}</span>
        </div>
      );
    case "transcript":
      return <div className="hud-line hud-line-transcript">{selected.text}</div>;
    default:
      return <div className="hud-center-placeholder" aria-hidden />;
  }
}

function HudShell() {
  const reduceMotion = useReducedMotion();
  const shellRef = useRef<HTMLDivElement>(null);
  const centerInput = useHudCenterInput();
  const overlayMode = useHudStore((s) => s.overlayMode);
  // Pin layout for this shell instance so exit fade keeps dictation chrome if mode flips mid-animation.
  const shellOverlayModeRef = useRef<HudOverlayMode | null>(null);
  if (shellOverlayModeRef.current === null) {
    shellOverlayModeRef.current = overlayMode;
  }
  const isDictation = shellOverlayModeRef.current === "dictation";
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
  const phaseLabel = selectPhaseLabel(phase);

  // Always show mic chrome while `listening`. `match-result` can arrive before `hud-phase`
  // advances; hiding chrome when `match` is set produced an empty frosted shell with no level
  // indicator if the phase event was missed or reordered.
  const showListeningChrome = phase === "listening";

  /**
   * Backdrop-filter / inset shadows can outlast opacity on WebView2. Strip those layers as soon as
   * the session leaves overlay phases — `executing` / `done` dismiss before `stopped`, so gating on
   * `stopped` alone left a rim fading on its own.
   */
  useLayoutEffect(() => {
    if (isHudOverlayShellActive(phase)) return;
    const el = shellRef.current;
    if (!el) return;
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

  const shellInitial = reduceMotion
    ? { opacity: 1, scale: 1 }
    : { opacity: 0, scale: 0.985 };

  const shellExit = reduceMotion ? { opacity: 0 } : { opacity: 0, scale: 1 };

  if (isDictation) {
    return (
      <motion.div
        ref={shellRef}
        className="hud-root hud-root--dictation"
        data-tauri-drag-region
        onPointerDown={handleHudDragPointerDown}
        role="region"
        aria-label="Dictation session"
        initial={shellInitial}
        animate={{ opacity: 1, scale: 1 }}
        exit={shellExit}
        transition={transition}
      >
        <div className="hud-body hud-body--dictation">
          <WaveformBars dictation />
        </div>
      </motion.div>
    );
  }

  return (
    <motion.div
      ref={shellRef}
      className="hud-root"
      data-tauri-drag-region
      onPointerDown={handleHudDragPointerDown}
      role="region"
      {...(phaseLabel
        ? { "aria-labelledby": "hud-phase-label" }
        : { "aria-label": "Voice session" })}
      initial={shellInitial}
      animate={{ opacity: 1, scale: 1 }}
      exit={shellExit}
      transition={transition}
    >
      <div
        className={
          showListeningChrome ? "hud-body" : "hud-body hud-body--solo"
        }
      >
        {phaseLabel && (
          <div className="hud-phase-label" id="hud-phase-label">
            {phaseLabel}
          </div>
        )}
        <div className="hud-transcript-wrap">
          <span className="hud-sr-only" role="status" aria-live="polite" aria-atomic="true">
            {srAnnounced}
          </span>
          <div className="hud-transcript-visual">
            <CenterContent input={centerInput} selected={selected} />
          </div>
        </div>
        {showListeningChrome && (
          <div className="hud-bottom-bar">
            <WaveformBars />
          </div>
        )}
      </div>
    </motion.div>
  );
}

/** Fade / pulse wrapper: only mount animated shell while HUD session is active. */
function HudBody() {
  const phase = useHudStore((s) => s.phase);
  const active = isHudOverlayShellActive(phase);

  // `schedule_hud_window_hide_when_still_dismissed` in Rust delays native hide until exit finishes.
  return (
    <AnimatePresence>
      {active ? <HudShell key="shell" /> : null}
    </AnimatePresence>
  );
}

export function HudPanel() {
  return (
    <div
      className="hud-panel-fill"
      data-tauri-drag-region
      onPointerDown={handleHudDragPointerDown}
    >
      <HudBody />
    </div>
  );
}
