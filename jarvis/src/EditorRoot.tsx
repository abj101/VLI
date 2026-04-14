import { NodeList } from "./components/editor/NodeList";
import "./EditorRoot.css";

export default function EditorRoot() {
  return (
    <main className="editor-root">
      <NodeList />
      <section className="editor-panel editor-panel-right">
        <header className="editor-panel-header">
          <h2>Node Form</h2>
        </header>
        <div className="editor-form-placeholder">Select a node or press + to create one.</div>
      </section>
    </main>
  );
}
