import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { HudPhase } from "./types";
import "./App.css";

export default function App() {
  const [phase, setPhase] = useState<HudPhase>("idle");

  useEffect(() => {
    let cancelled = false;

    void (async () => {
      try {
        const p = await invoke<HudPhase>("hud_get_phase");
        if (!cancelled) setPhase(p);
      } catch {
        // Web-only `npm run dev` without Tauri — ignore.
      }
    })();

    let unlisten: (() => void) | undefined;
    void listen<{ phase: HudPhase }>("hud-phase", (e) => {
      setPhase(e.payload.phase);
    }).then((fn) => {
      unlisten = fn;
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      void invoke("hud_dismiss").catch(() => {});
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div className="hud-root">
      <div className="hud-title">JARVIS</div>
      <div className="hud-phase">{phase}</div>
      <p className="hud-hint">Toggle: Ctrl+Shift+J · Esc stops</p>
      <div className="hud-actions">
        <button
          type="button"
          onClick={() => void invoke("hud_set_phase", { phase: "idle" })}
        >
          Idle (click-through)
        </button>
        <button
          type="button"
          onClick={() => void invoke("hud_set_phase", { phase: "listening" })}
        >
          Listening
        </button>
        <button
          type="button"
          className="btn-stop"
          onClick={() => void invoke("hud_dismiss")}
        >
          Stop
        </button>
      </div>
    </div>
  );
}
