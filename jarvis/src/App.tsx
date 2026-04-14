import { useEffect } from "react";
import { HudPanel } from "./components/hud/HudPanel";
import { subscribeHudIpc } from "./store/hudIpc";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

export default function App() {
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void subscribeHudIpc().then((u) => {
      unlisten = u;
    });
    return () => {
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

  return <HudPanel />;
}
