import { NodeList } from "./components/editor/NodeList";
import { NodeForm } from "./components/editor/NodeForm";
import { SettingsPanel } from "./components/editor/SettingsPanel";
import "./EditorRoot.css";
import { useState } from "react";

export default function EditorRoot() {
  const [settingsOpen, setSettingsOpen] = useState(false);

  return (
    <main className="editor-root">
      <header className="editor-root-header">
        <h1>JARVIS Command Editor</h1>
        <button
          type="button"
          className="editor-gear-btn"
          onClick={() => setSettingsOpen((open) => !open)}
          aria-label={settingsOpen ? "Close settings panel" : "Open settings panel"}
          aria-pressed={settingsOpen}
        >
          ⚙
        </button>
      </header>
      <div className="editor-root-content">
        <NodeList />
        <NodeForm />
      </div>
      {settingsOpen && <SettingsPanel onClose={() => setSettingsOpen(false)} />}
    </main>
  );
}
