import "@fontsource/work-sans/400.css";
import "@fontsource/work-sans/500.css";
import "@fontsource/work-sans/600.css";
import React from "react";
import ReactDOM from "react-dom/client";
import EditorRoot from "./EditorRoot";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <EditorRoot />
  </React.StrictMode>,
);
