import { describe, expect, it } from "vitest";
import { initialHudState, reduceHudState } from "./hudReducer";
import type { HudPhase } from "../types";

it("keeps the last phrase visible when the decoder resets its silence buffer", () => {
  const listening = reduceHudState(initialHudState, "hud-phase", {
    phase: "listening", session_id: 7,
  });
  const spoken = reduceHudState(listening, "transcript-update", {
    text: "open notepad", is_final: false, hud_session_id: 7,
  });
  const reset = reduceHudState(spoken, "transcript-update", {
    text: "", is_final: false, hud_session_id: 7, kind: "silence_reset",
  });
  expect(reset.transcript).toBe("open notepad");
});

describe("reduceHudState", () => {
  it("preserves an execution error through completion feedback", () => {
    const failed = { ...initialHudState, phase: "executing" as const, actionError: "Launch failed" };
    const done = reduceHudState(failed, "hud-phase", { phase: "done" });
    expect(done.actionError).toBe("Launch failed");
    const stopped = reduceHudState(done, "hud-phase", { phase: "stopped" });
    expect(stopped.actionError).toBeNull();
  });

  it("tracks dictation waiting and ignores stale waiting events", () => {
    const waiting = reduceHudState(initialHudState, "dictation-phase", {
      active: true, sessionId: 8, waitingForInput: true,
    });
    expect(waiting.dictationWaiting).toBe(true);
    const stale = reduceHudState(waiting, "dictation-phase", {
      active: true, sessionId: 7, waitingForInput: false,
    });
    expect(stale).toBe(waiting);
    const ready = reduceHudState(waiting, "dictation-phase", {
      active: true, sessionId: 8, waitingForInput: false,
    });
    expect(ready.dictationWaiting).toBe(false);
    const stopped = reduceHudState(waiting, "dictation-phase", {
      active: false, sessionId: 8,
    });
    expect(stopped.dictationWaiting).toBe(false);
  });
  it.each([
    ["Hey Jarvis, open notepad", "open notepad"],
    ["HEY, JARVIS. Open Discord.", "Open Discord."],
    ["Hey Jarvis.", ""],
    ["Jarvis, open notepad", "open notepad"],
    ["Okay Jervis, open notepad", "open notepad"],
    ["open the hey jarvis settings", "open the hey jarvis settings"],
    ["hey Sam, open notepad", "hey Sam, open notepad"],
  ])("hides the wake prefix in HUD transcript %j", (text, expected) => {
    const listening = reduceHudState(initialHudState, "hud-phase", {
      phase: "listening", session_id: 7,
    });
    for (const is_final of [false, true]) {
      const next = reduceHudState(listening, "transcript-update", {
        text, is_final, hud_session_id: 7,
      });
      expect(next.transcript).toBe(expected);
      expect(next.transcriptFinal).toBe(is_final);
    }
  });

  it("replays the dictation print for every inserted sentence, even repeats", () => {
    const once = reduceHudState(initialHudState, "dictation-phase", { active: true, sessionId: 3, text: "Hi." });
    const twice = reduceHudState(once, "dictation-phase", { active: true, sessionId: 3, text: "Hi." });
    const idle = reduceHudState(twice, "dictation-phase", { active: true, sessionId: 3 });
    expect(once.dictationPrint).toEqual({ text: "Hi.", id: 1 });
    expect(twice.dictationPrint.id).toBe(2);
    expect(idle.dictationPrint).toBe(twice.dictationPrint);
  });

  it("ignores the starting command's late status after switching to dictation", () => {
    const command = reduceHudState(initialHudState, "hud-phase", {
      phase: "executing", session_id: 7, overlay_mode: "command",
    });
    const dictation = reduceHudState(command, "hud-phase", {
      phase: "listening", session_id: 8, overlay_mode: "dictation",
    });
    const next = reduceHudState(dictation, "action-status", { text: "Started dictation" });
    expect(next).toBe(dictation);
    expect(next.phase).toBe("listening");
    expect(next.actionText).toBeNull();
  });

  it("applies hud-phase and replaces phase", () => {
    const next = reduceHudState(initialHudState, "hud-phase", {
      phase: "listening" as HudPhase,
    });
    expect(next.phase).toBe("listening");
  });

  it("ignores hud-phase from an older session_id", () => {
    const stopped = {
      ...initialHudState,
      phase: "stopped" as const,
      sessionId: 8,
    };
    const next = reduceHudState(stopped, "hud-phase", {
      phase: "listening",
      session_id: 7,
    });
    expect(next).toBe(stopped);
  });

  it("on listening, clears prior transcript, match, action, errors, amplitude", () => {
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
        remainder: "",
      },
      actionText: "Opening…",
      actionError: "launch failed",
      amplitude: 0.8,
      audioError: "mic failed",
    };
    const next = reduceHudState(dirty, "hud-phase", { phase: "listening" });
    expect(next.transcript).toBe("");
    expect(next.transcriptFinal).toBe(false);
    expect(next.match).toBeNull();
    expect(next.actionText).toBeNull();
    expect(next.actionError).toBeNull();
    expect(next.amplitude).toBe(0);
    expect(next.audioError).toBeNull();
  });

  it("on executing, clears match and action so done/stopped never resurrects UI", () => {
    const dirty = {
      ...initialHudState,
      phase: "listening" as const,
      match: {
        node_id: "n1",
        matched_phrase: "open notepad",
        span_start: 0,
        span_end: 12,
        remainder: "",
      },
      actionText: "Opening…",
      transcript: "open notepad",
    };
    const next = reduceHudState(dirty, "hud-phase", { phase: "executing" });
    expect(next.match).toBeNull();
    expect(next.actionText).toBeNull();
    expect(next.transcript).toBe("");
  });

  it("on awaiting_input, clears stale match and transcript", () => {
    const dirty = {
      ...initialHudState,
      phase: "executing" as const,
      transcript: "open github",
      transcriptFinal: true,
      match: {
        node_id: "n1",
        matched_phrase: "open github",
        span_start: 0,
        span_end: 11,
        remainder: "",
      },
    };
    const next = reduceHudState(dirty, "hud-phase", { phase: "awaiting_input" });
    expect(next.match).toBeNull();
    expect(next.transcript).toBe("");
    expect(next.transcriptFinal).toBe(false);
  });

  it("ignores transcript-update from a stale hud_session_id", () => {
    const listening = {
      ...initialHudState,
      phase: "listening" as const,
      sessionId: 5,
      transcript: "hello",
    };
    const next = reduceHudState(listening, "transcript-update", {
      text: "stale",
      is_final: false,
      hud_session_id: 4,
    });
    expect(next).toBe(listening);
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

  it("clears match when transcript updates while listening", () => {
    const withMatch = {
      ...initialHudState,
      phase: "listening" as const,
      transcript: "open notepad",
      match: {
        node_id: "n1",
        matched_phrase: "open notepad",
        span_start: 0,
        span_end: 12,
        remainder: "",
      },
    };
    const next = reduceHudState(withMatch, "transcript-update", {
      text: "open notepad now",
      is_final: false,
    });
    expect(next.match).toBeNull();
  });

  it("keeps match when transcript updates after listening (late STT final)", () => {
    const withMatch = {
      ...initialHudState,
      phase: "executing" as const,
      transcript: "open notepad",
      match: {
        node_id: "n1",
        matched_phrase: "open notepad",
        span_start: 0,
        span_end: 12,
        remainder: "",
      },
    };
    const next = reduceHudState(withMatch, "transcript-update", {
      text: "open notepad",
      is_final: true,
    });
    expect(next.match).toEqual(withMatch.match);
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
        remainder: "",
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
    expect(s.actionError).toBeNull();
  });

  it("applies action-error message", () => {
    const s = reduceHudState(initialHudState, "action-error", {
      message: "launch failed",
    });
    expect(s.actionError).toBe("launch failed");
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
