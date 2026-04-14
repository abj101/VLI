import { motion } from "framer-motion";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useRef } from "react";
import { useHudStore } from "../../store/hudStore";
import type { MatchResult } from "../../types";
import type { HudPhase } from "../../types";

const LISTENING_PHASES: HudPhase[] = [
  "listening",
  "matched",
  "executing",
  "awaiting_input",
  "done",
];

function isActiveHudPhase(phase: HudPhase): boolean {
  return LISTENING_PHASES.includes(phase);
}

/** Shimmer + pulse while Rust is running matched command chain (not during done fade). */
function useShowAgentPulse(phase: HudPhase): boolean {
  return (
    phase === "matched" ||
    phase === "executing" ||
    phase === "awaiting_input"
  );
}

function WaveformBars() {
  const phase = useHudStore((s) => s.phase);
  const amplitude = useHudStore((s) => s.amplitude);
  const active = phase === "listening";
  const level = active ? amplitude : 0;

  const bars = useMemo(() => [0, 1, 2, 3, 4, 5, 6], []);

  return (
    <div className="hud-waveform" aria-hidden>
      {bars.map((i) => {
        const wave = 0.25 + 0.75 * Math.sin((i / 6) * Math.PI + level * 2.4);
        const h = 6 + level * wave * 22;
        return (
          <motion.div
            key={i}
            className="hud-waveform-bar"
            animate={{
              height: active ? h : 3,
              opacity: active ? 0.55 + level * 0.45 : 0.12,
            }}
            transition={{ type: "spring", stiffness: 520, damping: 28 }}
          />
        );
      })}
    </div>
  );
}

function StopHudButton() {
  const phase = useHudStore((s) => s.phase);
  const listening = phase === "listening";

  const onStop = () => {
    void invoke("hud_dismiss").catch(() => {});
  };

  return (
    <motion.button
      type="button"
      className="hud-stop"
      aria-label="Stop"
      onClick={onStop}
      animate={{ opacity: listening ? 1 : 0.25, scale: listening ? 1 : 0.92 }}
      transition={{ duration: 0.18 }}
      disabled={!listening}
    >
      <motion.span
        className="hud-stop-ring"
        animate={
          listening
            ? { borderColor: ["rgba(255,255,255,0.35)", "rgba(255,255,255,1)", "rgba(255,255,255,0.35)"] }
            : { borderColor: "rgba(255,255,255,0.35)" }
        }
        transition={
          listening
            ? { duration: 1.5, repeat: Infinity, ease: "easeInOut" }
            : { duration: 0.2 }
        }
      />
      <span className="hud-stop-inner" />
    </motion.button>
  );
}

function RecognizedPhrase() {
  const phase = useHudStore((s) => s.phase);
  const match = useHudStore((s) => s.match);
  const pulse = useShowAgentPulse(phase);

  if (!match) return null;

  return (
    <motion.div
      key={`${match.node_id}-${match.matched_phrase}`}
      className="hud-recognized-wrap"
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.38, ease: "easeOut" }}
    >
      <span
        className={
          pulse ? "hud-recognized-text hud-recognized-text--pulse" : "hud-recognized-text"
        }
      >
        {match.matched_phrase}
      </span>
    </motion.div>
  );
}

type CenterSelectorInput = {
  phase: HudPhase;
  transcript: string;
  match: MatchResult | null;
  actionText: string | null;
  actionError: string | null;
  audioError: string | null;
};

type CenterSelectorResult =
  | { kind: "error"; text: string }
  | { kind: "match" }
  | { kind: "action"; text: string }
  | { kind: "transcript"; text: string }
  | { kind: "placeholder" };

export function selectCenterContent(
  input: CenterSelectorInput,
): CenterSelectorResult {
  if (
    (input.phase === "listening" || input.phase === "awaiting_input") &&
    input.audioError
  ) {
    return { kind: "error", text: input.audioError };
  }

  if (input.actionError) {
    return { kind: "error", text: input.actionError };
  }

  if (input.match) {
    return { kind: "match" };
  }

  const transcript = input.transcript.trim();
  if (input.phase === "awaiting_input" && transcript.length > 0) {
    return { kind: "transcript", text: input.transcript };
  }

  if (
    input.actionText &&
    (input.phase === "matched" ||
      input.phase === "executing" ||
      input.phase === "awaiting_input" ||
      input.phase === "done")
  ) {
    return { kind: "action", text: input.actionText };
  }

  if (transcript.length > 0) {
    return { kind: "transcript", text: input.transcript };
  }

  return { kind: "placeholder" };
}

export function selectPhaseLabel(phase: HudPhase): string | null {
  switch (phase) {
    case "listening":
      return null;
    case "matched":
      return "Matched";
    case "awaiting_input":
      return null;
    case "executing":
      return "Executing";
    case "done":
      return "Done";
    case "stopped":
      return "Stopped";
    default:
      return null;
  }
}

function CenterContent() {
  const phase = useHudStore((s) => s.phase);
  const transcript = useHudStore((s) => s.transcript);
  const match = useHudStore((s) => s.match);
  const actionText = useHudStore((s) => s.actionText);
  const actionError = useHudStore((s) => s.actionError);
  const audioError = useHudStore((s) => s.audioError);
  const selected = selectCenterContent({
    phase,
    transcript,
    match,
    actionText,
    actionError,
    audioError,
  });

  switch (selected.kind) {
    case "error":
      return (
        <div className="hud-line hud-line-error" role="alert">
          {selected.text}
        </div>
      );
    case "match":
      return <RecognizedPhrase />;
    case "action":
      return (
        <div className="hud-line hud-line-action">
          <span className="hud-app-tag">JARVIS</span>
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
  const phase = useHudStore((s) => s.phase);
  const match = useHudStore((s) => s.match);
  const phaseLabel = selectPhaseLabel(phase);

  const showListeningChrome =
    phase === "listening" && !match;

  const rootOpacity =
    phase === "idle" || phase === "stopped"
      ? 0
      : phase === "done"
        ? 0
        : 1;

  const transition = useMemo(() => {
    if (phase === "done") {
      return { duration: 0.55, ease: "easeInOut" as const };
    }
    if (phase === "stopped") {
      return { duration: 0.14, ease: "easeOut" as const };
    }
    return { duration: 0.4, ease: "easeOut" as const };
  }, [phase]);

  return (
    <motion.div
      className="hud-root"
      initial={{ opacity: 0 }}
      animate={{ opacity: rootOpacity }}
      transition={transition}
    >
      {showListeningChrome && (
        <motion.div
          className="hud-title"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          transition={{ duration: 0.35, delay: 0.05 }}
        >
          JARVIS
        </motion.div>
      )}
      <div
        className={
          showListeningChrome ? "hud-body" : "hud-body hud-body--solo"
        }
      >
        {phaseLabel && <div className="hud-phase-label">{phaseLabel}</div>}
        <div className="hud-transcript-wrap">
          <CenterContent />
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
