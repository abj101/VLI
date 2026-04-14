import { NodeList } from "./components/editor/NodeList";
import { NodeForm } from "./components/editor/NodeForm";
import "./EditorRoot.css";

export default function EditorRoot() {
  return (
    <main className="editor-root">
      <NodeList />
      <NodeForm />
    </main>
  );
}
