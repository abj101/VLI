import { motion, useReducedMotion } from "framer-motion";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useRef } from "react";
import { useShallow } from "zustand/react/shallow";
import { useDebounced } from "../../hooks/useDebounced";
import { useHudStore } from "../../store/hudStore";
import type { HudPhase } from "../../types";
import {
  announcableText,
  selectCenterContent,
  selectPhaseLabel,
  type CenterSelectorInput,
  type CenterSelectorResult,
} from "./HudPanel.logic";

const LISTENING_PHASES: HudPhase[] = [
  "listening",
  "matched",
  "executing",
  "awaiting_input",
  "done",
];

/** Trailing debounce (ms) for streaming transcript in the SR-only live region. */
const TRANSCRIPT_ANNOUNCE_DEBOUNCE_MS = 380;

/** Fixed sleeve height for bars — motion uses `scaleY` only (no layout thrash). */
const WAVE_BAR_SLEEVE_PX = 28;

function isActiveHudPhase(phase: HudPhase): boolean {
  return LISTENING_PHASES.includes(phase);
}

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

function WaveformBars() {
  const reduceMotion = useReducedMotion();
  const phase = useHudStore((s) => s.phase);
  const amplitude = useHudStore((s) => s.amplitude);
  const active = phase === "listening";
  const level = active ? amplitude : 0;

  const bars = useMemo(() => [0, 1, 2, 3, 4, 5, 6], []);

  const barTransition = useMemo(
    () =>
      reduceMotion
        ? { duration: 0.12, ease: "easeOut" as const }
        : { duration: 0.22, ease: [0.22, 1, 0.36, 1] as [number, number, number, number] },
    [reduceMotion],
  );

  return (
    <div className="hud-waveform" aria-hidden>
      {bars.map((i) => {
        const wave = 0.25 + 0.75 * Math.sin((i / 6) * Math.PI + level * 2.4);
        const rawH = 6 + level * wave * 22;
        const scaleY = active
          ? Math.max(0.14, Math.min(1, rawH / WAVE_BAR_SLEEVE_PX))
          : 0.14;

        return (
          <motion.div
            key={i}
            className="hud-waveform-bar"
            style={{
              height: WAVE_BAR_SLEEVE_PX,
              transformOrigin: "bottom center",
            }}
            initial={false}
            animate={{
              scaleY,
              opacity: active ? 0.55 + level * 0.45 : 0.12,
            }}
            transition={barTransition}
          />
        );
      })}
    </div>
  );
}

function StopHudButton() {
  const reduceMotion = useReducedMotion();
  const phase = useHudStore((s) => s.phase);
  const listening = phase === "listening";

  const onStop = () => {
    void invoke("hud_dismiss").catch(() => {});
  };

  const ringClass =
    listening && !reduceMotion
      ? "hud-stop-ring hud-stop-ring--pulse"
      : "hud-stop-ring";

  return (
    <motion.button
      type="button"
      className="hud-stop"
      aria-label="Stop listening. Same as Escape."
      onClick={onStop}
      animate={{ opacity: listening ? 1 : 0.25, scale: listening ? 1 : 0.92 }}
      transition={{ duration: reduceMotion ? 0.08 : 0.18 }}
      disabled={!listening}
    >
      <span className={ringClass} />
      <span className="hud-stop-inner" />
    </motion.button>
  );
}

function RecognizedPhrase({
  phrase,
  motionKey,
}: {
  phrase: string;
  motionKey: string;
}) {
  const reduceMotion = useReducedMotion();

  return (
    <motion.div
      key={motionKey}
      className="hud-recognized-wrap"
      initial={reduceMotion ? { opacity: 1, y: 0 } : { opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={
        reduceMotion
          ? { duration: 0.01 }
          : { duration: 0.38, ease: "easeOut" as const }
      }
    >
      <span className="hud-recognized-text">
        {phrase}
      </span>
    </motion.div>
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
  const centerInput = useHudCenterInput();
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
  const match = centerInput.match;
  const phaseLabel = selectPhaseLabel(phase);

  const showListeningChrome = phase === "listening" && !match;

  const rootOpacity =
    phase === "idle" || phase === "stopped"
      ? 0
      : phase === "done"
        ? 0
        : 1;

  const transition = useMemo(() => {
    if (reduceMotion) {
      return { duration: 0.12, ease: "easeOut" as const };
    }
    if (phase === "done") {
      return { duration: 0.55, ease: "easeInOut" as const };
    }
    if (phase === "stopped") {
      return { duration: 0.14, ease: "easeOut" as const };
    }
    return { duration: 0.4, ease: "easeOut" as const };
  }, [phase, reduceMotion]);

  return (
    <motion.div
      className="hud-root"
      role="region"
      {...(phaseLabel
        ? { "aria-labelledby": "hud-phase-label" }
        : { "aria-label": "Voice session" })}
      initial={{ opacity: 0 }}
      animate={{ opacity: rootOpacity }}
      transition={transition}
    >
      {showListeningChrome && (
        <motion.div
          className="hud-title"
          initial={reduceMotion ? { opacity: 1 } : { opacity: 0 }}
          animate={{ opacity: 1 }}
          transition={
            reduceMotion
              ? { duration: 0.01 }
              : { duration: 0.35, delay: 0.05 }
          }
        >
          JARVIS
        </motion.div>
      )}
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
            <StopHudButton />
          </div>
        )}
      </div>
    </motion.div>
  );
}

/** Fade / pulse wrapper: only mount animated shell while HUD session is active. */
function HudBody() {
  const phase = useHudStore((s) => s.phase);
  const active = isActiveHudPhase(phase);

  if (!active) {
    return null;
  }

  return <HudShell />;
}

export function HudPanel() {
  const applyIpc = useHudStore((s) => s.applyIpc);
  const mounted = useRef(false);

  useEffect(() => {
    mounted.current = true;
    void (async () => {
      try {
        const p = await invoke<HudPhase>("hud_get_phase");
        if (mounted.current) applyIpc("hud-phase", { phase: p });
      } catch {
        /* web-only */
      }
    })();
    return () => {
      mounted.current = false;
    };
  }, [applyIpc]);

  return (
    <div className="hud-panel-fill">
      <HudBody />
    </div>
  );
}
