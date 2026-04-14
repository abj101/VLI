import { describe, expect, it } from "vitest";
import {
  initialHudState,
  reduceHudState,
  sliceTranscriptBySpan,
} from "./hudReducer";
import type { HudPhase } from "../types";

describe("reduceHudState", () => {
  it("applies hud-phase and replaces phase", () => {
    const next = reduceHudState(initialHudState, "hud-phase", {
      phase: "listening" as HudPhase,
    });
    expect(next.phase).toBe("listening");
  });

  it("on listening, clears prior transcript, match, action, amplitude", () => {
    const dirty = {
      ...initialHudState,
      phase: "done" as const,
      transcript: "open notepad",
      transcriptFinal: true,
      match: {
        node_id: "n1",
        matched_phrase: "open notepad",
        span_start: 0,
        span_end: 12,
      },
      actionText: "Opening…",
      amplitude: 0.8,
      audioError: "mic failed",
    };
    const next = reduceHudState(dirty, "hud-phase", { phase: "listening" });
    expect(next.transcript).toBe("");
    expect(next.transcriptFinal).toBe(false);
    expect(next.match).toBeNull();
    expect(next.actionText).toBeNull();
    expect(next.amplitude).toBe(0);
    expect(next.audioError).toBeNull();
  });

  it("applies transcript-update text and is_final", () => {
    let s = reduceHudState(initialHudState, "transcript-update", {
      text: "open",
      is_final: false,
    });
    expect(s.transcript).toBe("open");
    expect(s.transcriptFinal).toBe(false);
    s = reduceHudState(s, "transcript-update", {
      text: "open notepad",
      is_final: true,
    });
    expect(s.transcript).toBe("open notepad");
    expect(s.transcriptFinal).toBe(true);
  });

  it("clears match when transcript updates", () => {
    const withMatch = {
      ...initialHudState,
      transcript: "open notepad",
      match: {
        node_id: "n1",
        matched_phrase: "open notepad",
        span_start: 0,
        span_end: 12,
      },
    };
    const next = reduceHudState(withMatch, "transcript-update", {
      text: "open notepad now",
      is_final: false,
    });
    expect(next.match).toBeNull();
  });

  it("applies match-result", () => {
    const s = reduceHudState(
      { ...initialHudState, transcript: "please open notepad" },
      "match-result",
      {
        node_id: "seed-1",
        matched_phrase: "open notepad",
        span_start: 7,
        span_end: 19,
      },
    );
    expect(s.match?.span_start).toBe(7);
    expect(s.match?.span_end).toBe(19);
  });

  it("applies action-status", () => {
    const s = reduceHudState(initialHudState, "action-status", {
      text: "Opening Notepad…",
    });
    expect(s.actionText).toBe("Opening Notepad…");
  });

  it("applies amplitude-update and clamps to 0..1", () => {
    let s = reduceHudState(initialHudState, "amplitude-update", {
      amplitude: 1.5,
    });
    expect(s.amplitude).toBe(1);
    s = reduceHudState(initialHudState, "amplitude-update", {
      amplitude: -2,
    });
    expect(s.amplitude).toBe(0);
  });

  it("applies audio-error message", () => {
    const s = reduceHudState(initialHudState, "audio-error", {
      message: "Whisper model missing",
    });
    expect(s.audioError).toBe("Whisper model missing");
  });
});

describe("sliceTranscriptBySpan", () => {
  it("splits text by span for highlight rendering", () => {
    const r = sliceTranscriptBySpan("please open notepad", 7, 19);
    expect(r.before).toBe("please ");
    expect(r.match).toBe("open notepad");
    expect(r.after).toBe("");
  });

  it("clamps span to string length", () => {
    const r = sliceTranscriptBySpan("hi", 0, 100);
    expect(r.match).toBe("hi");
    expect(r.after).toBe("");
  });
});
