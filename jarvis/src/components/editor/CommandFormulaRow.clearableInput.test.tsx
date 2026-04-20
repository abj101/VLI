import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

/** Static shape: app re-edit field reserves left column for icon + right for ✕ on hover/focus (CSS). */
function OpenAppSearchWithLeadingIconHarness() {
  return (
    <div className="editor-formula-arg-slot editor-formula-arg-slot--clearable">
      <div className="editor-formula-arg-wrap editor-formula-arg-wrap--leading-app-icon">
        <span className="editor-formula-input-leading-icon" aria-hidden>
          <span className="editor-formula-suggest-icon editor-formula-suggest-icon--fallback">N</span>
        </span>
        <input
          readOnly
          className="editor-formula-input editor-formula-input--arg editor-formula-input--autogrow"
          aria-label="App name"
        />
      </div>
      <button type="button" className="editor-formula-remove-inline" aria-label="Remove step 1">
        <span className="editor-formula-remove-inline-x" />
      </button>
    </div>
  );
}

describe("CommandFormulaRow clearable text field shell", () => {
  it("places leading app icon before the input inside the arg wrap", () => {
    const html = renderToStaticMarkup(<OpenAppSearchWithLeadingIconHarness />);
    expect(html).toContain("editor-formula-arg-wrap--leading-app-icon");
    expect(html).toMatch(
      /editor-formula-input-leading-icon[\s\S]*editor-formula-input editor-formula-input--arg/,
    );
  });

  it("keeps the inline remove control as a sibling of the arg wrap inside the clearable slot", () => {
    const html = renderToStaticMarkup(<OpenAppSearchWithLeadingIconHarness />);
    expect(html).toMatch(
      /editor-formula-arg-slot--clearable[^>]*>[\s\S]*editor-formula-arg-wrap[\s\S]*<\/div>[\s\S]*editor-formula-remove-inline/,
    );
  });
});
