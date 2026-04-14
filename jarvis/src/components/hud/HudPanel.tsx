import { motion } from "framer-motion";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useRef } from "react";
import { useHudStore } from "../../store/hudStore";
import { sliceTranscriptBySpan } from "../../store/hudReducer";
import type { HudPhase } from "../../types";

const WORD_SPLIT = /(\s+)/;

function tokenizeWords(text: string): string[] {
  if (!text) return [];
  return text.split(WORD_SPLIT).filter((t) => t.length > 0);
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
            ? { borderColor: ["rgba(255,70,70,0.4)", "rgba(255,70,70,1)", "rgba(255,70,70,0.4)"] }
            : { borderColor: "rgba(255,70,70,0.35)" }
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

function TranscriptBlock() {
  const phase = useHudStore((s) => s.phase);
  const transcript = useHudStore((s) => s.transcript);
  const match = useHudStore((s) => s.match);
  const actionText = useHudStore((s) => s.actionText);

  const showAction =
    (phase === "executing" || phase === "awaiting_input") &&
    actionText &&
    actionText.length > 0;

  if (showAction) {
    return (
      <motion.div
        key="action"
        className="hud-line hud-line-action"
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        transition={{ duration: 0.22 }}
      >
        {actionText}
      </motion.div>
    );
  }

  if (!transcript) {
    return (
      <div className="hud-line hud-line-muted">
        {phase === "listening" ? "Listening…" : "\u00a0"}
      </div>
    );
  }

  if (match) {
    const { before, match: mid, after } = sliceTranscriptBySpan(
      transcript,
      match.span_start,
      match.span_end,
    );
    const dimSurround = phase === "matched";
    return (
      <div className="hud-line hud-line-transcript">
        {before && (
          <motion.span
            animate={{ opacity: dimSurround ? 0.35 : 1 }}
            transition={{ duration: 0.3 }}
          >
            {before}
          </motion.span>
        )}
        <motion.span
          className="hud-match"
          animate={
            phase === "matched"
              ? { scale: 1.05, y: -4 }
              : { scale: 1, y: 0 }
          }
          transition={{ type: "tween", ease: "easeOut", duration: 0.2 }}
        >
          {mid}
        </motion.span>
        {after && (
          <motion.span
            animate={{ opacity: dimSurround ? 0.35 : 1 }}
            transition={{ duration: 0.3 }}
          >
            {after}
          </motion.span>
        )}
      </div>
    );
  }

  const tokens = tokenizeWords(transcript);
  return (
    <div className="hud-line hud-line-transcript">
      {tokens.map((tok, idx) => (
        <motion.span
          key={`${idx}-${tok}`}
          className="hud-word"
          initial={{ opacity: 0.15 }}
          animate={{ opacity: 1 }}
          transition={{ duration: 0.12, delay: Math.min(idx, 24) * 0.02 }}
        >
          {tok}
        </motion.span>
      ))}
    </div>
  );
}

function HudBody() {
  const phase = useHudStore((s) => s.phase);

  const opacity =
    phase === "done"
      ? [1, 1, 0]
      : phase === "stopped"
        ? 0
        : 1;

  const transition =
    phase === "done"
      ? { duration: 0.5, times: [0, 0.6, 1], ease: "easeInOut" as const }
      : phase === "stopped"
        ? { duration: 0.15, ease: "easeOut" as const }
        : { duration: 0.2 };

  return (
    <motion.div
      className="hud-body"
      animate={{ opacity }}
      transition={transition}
    >
      <div className="hud-row-top">
        <WaveformBars />
        <StopHudButton />
      </div>
      <TranscriptBlock />
    </motion.div>
  );
}

const MOCK_PHASES: HudPhase[] = [
  "idle",
  "listening",
  "matched",
  "executing",
  "awaiting_input",
  "done",
  "stopped",
];

/** Dev-only: drive Rust phase + local IPC for transcript/match/action/amplitude. */
export function MockHudDemoBar() {
  const applyIpc = useHudStore((s) => s.applyIpc);

  const setPhase = (p: HudPhase) => {
    void invoke("hud_set_phase", { phase: p }).catch(() => {});
  };

  return (
    <div className="hud-mock">
      <div className="hud-mock-title">Mock IPC (dev)</div>
      <div className="hud-mock-row">
        {MOCK_PHASES.map((p) => (
          <button key={p} type="button" onClick={() => setPhase(p)}>
            {p}
          </button>
        ))}
      </div>
      <div className="hud-mock-row">
        <button
          type="button"
          onClick={() =>
            applyIpc("transcript-update", {
              text: "please open notepad",
              is_final: false,
            })
          }
        >
          transcript
        </button>
        <button
          type="button"
          onClick={() =>
            applyIpc("match-result", {
              node_id: "seed-1",
              matched_phrase: "open notepad",
              span_start: 7,
              span_end: 19,
            })
          }
        >
          match
        </button>
        <button
          type="button"
          onClick={() =>
            applyIpc("action-status", { text: "Opening Notepad…" })
          }
        >
          action
        </button>
        <button
          type="button"
          onClick={() => applyIpc("amplitude-update", { amplitude: 0.72 })}
        >
          amp
        </button>
      </div>
    </div>
  );
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
    <div className="hud-root">
      <div className="hud-title">JARVIS</div>
      <HudBody />
      <p className="hud-hint">Toggle: Ctrl+Shift+J · Esc stops</p>
      {import.meta.env.DEV ? <MockHudDemoBar /> : null}
    </div>
  );
}
