import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useState } from "react";
import type {
  NewToolDefinitionPayload,
  OpenTargetPreview,
  ToolDefinitionPayload,
  ToolParameter,
} from "../../types";
import { formatUserError } from "../../utils/userErrors";
import { defaultUserToolDraft, formatOpenTargetPreview } from "./toolsTab.logic";

function toolToDraft(tool: ToolDefinitionPayload): NewToolDefinitionPayload {
  return {
    name: tool.name,
    display_name: tool.display_name,
    description: tool.description,
    parameters: tool.parameters.map((p) => ({ ...p })),
    actions: [...tool.actions],
    enabled: tool.enabled,
    builtin: tool.builtin,
  };
}

function ParameterRow({
  param,
  index,
  readOnly,
  onChange,
  onRemove,
}: {
  param: ToolParameter;
  index: number;
  readOnly: boolean;
  onChange: (index: number, next: ToolParameter) => void;
  onRemove: (index: number) => void;
}) {
  return (
    <div className="editor-tools-param-row">
      <input
        type="text"
        className="editor-formula-input"
        value={param.name}
        readOnly={readOnly}
        placeholder="name"
        aria-label={`Parameter ${index + 1} name`}
        onChange={(e) => onChange(index, { ...param, name: e.target.value })}
      />
      <select
        className="editor-formula-input"
        value={param.param_type}
        disabled={readOnly}
        aria-label={`Parameter ${index + 1} type`}
        onChange={(e) => onChange(index, { ...param, param_type: e.target.value })}
      >
        <option value="string">string</option>
        <option value="enum">enum</option>
      </select>
      <label className="editor-tools-param-required">
        <input
          type="checkbox"
          checked={param.required}
          disabled={readOnly}
          onChange={(e) => onChange(index, { ...param, required: e.target.checked })}
        />
        required
      </label>
      {param.param_type === "enum" ? (
        <input
          type="text"
          className="editor-formula-input editor-tools-param-enum"
          value={(param.enum_values ?? []).join(", ")}
          readOnly={readOnly}
          placeholder="left_half, right_half"
          aria-label={`Parameter ${index + 1} enum values`}
          onChange={(e) =>
            onChange(index, {
              ...param,
              enum_values: e.target.value
                .split(",")
                .map((s) => s.trim())
                .filter(Boolean),
            })
          }
        />
      ) : null}
      {!readOnly ? (
        <button type="button" className="editor-tools-param-remove" onClick={() => onRemove(index)}>
          Remove
        </button>
      ) : null}
    </div>
  );
}

export function ToolsTab() {
  const [tools, setTools] = useState<ToolDefinitionPayload[]>([]);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [draft, setDraft] = useState<NewToolDefinitionPayload | null>(null);
  const [creating, setCreating] = useState(false);
  const [errorText, setErrorText] = useState<string | null>(null);
  const [testUtterance, setTestUtterance] = useState("open brave on the left");
  const [preview, setPreview] = useState<OpenTargetPreview | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);

  const selected = useMemo(
    () => tools.find((t) => t.id === selectedId) ?? null,
    [tools, selectedId],
  );

  const loadTools = useCallback(async () => {
    const list = await invoke<ToolDefinitionPayload[]>("list_tools");
    setTools(list);
    return list;
  }, []);

  useEffect(() => {
    let mounted = true;
    void loadTools()
      .then((list) => {
        if (!mounted || list.length === 0) return;
        setSelectedId((prev) => prev ?? list[0]?.id ?? null);
      })
      .catch((err: unknown) => {
        if (mounted) setErrorText(formatUserError(err, "Could not load tools."));
      });
    return () => {
      mounted = false;
    };
  }, [loadTools]);

  useEffect(() => {
    if (creating) {
      setDraft(defaultUserToolDraft());
      return;
    }
    if (selected) {
      setDraft(toolToDraft(selected));
    } else {
      setDraft(null);
    }
  }, [selected, creating]);

  const saveDraft = async () => {
    if (!draft) return;
    try {
      if (creating) {
        const created = await invoke<ToolDefinitionPayload>("create_tool", { tool: draft });
        const list = await loadTools();
        setCreating(false);
        setSelectedId(created.id);
        setTools(list);
      } else if (selected) {
        const saved = await invoke<ToolDefinitionPayload>("update_tool", {
          id: selected.id,
          tool: draft,
        });
        setTools((prev) => prev.map((t) => (t.id === saved.id ? saved : t)));
      }
      setErrorText(null);
    } catch (err: unknown) {
      setErrorText(formatUserError(err, "Could not save tool."));
    }
  };

  const runPreview = async () => {
    setPreviewError(null);
    try {
      const result = await invoke<OpenTargetPreview>("preview_open_target", {
        utterance: testUtterance,
      });
      setPreview(result);
    } catch (err: unknown) {
      setPreview(null);
      setPreviewError(formatUserError(err, "Preview failed."));
    }
  };

  const previewLines = preview ? formatOpenTargetPreview(preview) : null;
  const readOnlyBuiltin = Boolean(draft?.builtin && !creating);

  return (
    <div className="editor-tools-tab">
      <header className="editor-commands-toolbar editor-tools-toolbar">
        <h2 className="editor-tools-title">Tools</h2>
        <button
          type="button"
          className="editor-commands-add"
          aria-label="Add tool"
          onClick={() => {
            setCreating(true);
            setSelectedId(null);
          }}
        >
          +
        </button>
      </header>

      {errorText ? (
        <div className="editor-inline-toast" role="alert">
          {errorText}
        </div>
      ) : null}

      <div className="editor-tools-layout">
        <ul className="editor-tools-list" aria-label="Tool list">
          {tools.map((tool) => (
            <li key={tool.id}>
              <button
                type="button"
                className={`editor-tools-list-btn${selectedId === tool.id && !creating ? " is-active" : ""}`}
                onClick={() => {
                  setCreating(false);
                  setSelectedId(tool.id);
                }}
              >
                <span className="editor-tools-list-name">{tool.display_name}</span>
                <span className="editor-tools-list-meta">
                  {tool.builtin ? "builtin" : "user"} · {tool.name}
                </span>
              </button>
            </li>
          ))}
          {creating ? (
            <li>
              <button type="button" className="editor-tools-list-btn is-active">
                <span className="editor-tools-list-name">New tool</span>
              </button>
            </li>
          ) : null}
        </ul>

        <div className="editor-tools-editor">
          {draft ? (
            <>
              <label className="editor-tools-field">
                <span>Machine name</span>
                <input
                  type="text"
                  className="editor-formula-input"
                  value={draft.name}
                  readOnly={readOnlyBuiltin}
                  onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                />
              </label>
              <label className="editor-tools-field">
                <span>Display name</span>
                <input
                  type="text"
                  className="editor-formula-input"
                  value={draft.display_name}
                  readOnly={readOnlyBuiltin}
                  onChange={(e) => setDraft({ ...draft, display_name: e.target.value })}
                />
              </label>
              <label className="editor-tools-field">
                <span>Description</span>
                <textarea
                  className="editor-formula-input editor-tools-textarea"
                  value={draft.description}
                  onChange={(e) => setDraft({ ...draft, description: e.target.value })}
                />
              </label>

              <div className="editor-tools-section">
                <div className="editor-tools-section-head">
                  <h3>Parameters</h3>
                  {!readOnlyBuiltin ? (
                    <button
                      type="button"
                      className="editor-tools-inline-btn"
                      onClick={() =>
                        setDraft({
                          ...draft,
                          parameters: [
                            ...draft.parameters,
                            {
                              name: "",
                              param_type: "string",
                              required: false,
                              enum_values: [],
                            },
                          ],
                        })
                      }
                    >
                      Add parameter
                    </button>
                  ) : null}
                </div>
                {draft.parameters.map((param, index) => (
                  <ParameterRow
                    key={`${param.name}-${index}`}
                    param={param}
                    index={index}
                    readOnly={readOnlyBuiltin}
                    onChange={(i, next) => {
                      const parameters = [...draft.parameters];
                      parameters[i] = next;
                      setDraft({ ...draft, parameters });
                    }}
                    onRemove={(i) => {
                      setDraft({
                        ...draft,
                        parameters: draft.parameters.filter((_, idx) => idx !== i),
                      });
                    }}
                  />
                ))}
              </div>

              {draft.name === "open_target" ? (
                <div className="editor-tools-section">
                  <h3>Placement</h3>
                  <p className="editor-tools-hint">
                    Optional <code>placement</code> enum:{" "}
                    {(draft.parameters.find((p) => p.name === "placement")?.enum_values ?? []).join(
                      ", ",
                    ) || "left_half, right_half, maximize"}
                    . Parsed from utterances like &quot;on the left&quot;.
                  </p>
                </div>
              ) : null}

              <div className="editor-tools-actions-row">
                <button type="button" className="editor-formula-plus editor-formula-plus--labeled" onClick={() => void saveDraft()}>
                  Save tool
                </button>
                {creating ? (
                  <button
                    type="button"
                    className="editor-tools-inline-btn"
                    onClick={() => {
                      setCreating(false);
                      setSelectedId(tools[0]?.id ?? null);
                    }}
                  >
                    Cancel
                  </button>
                ) : null}
              </div>
            </>
          ) : (
            <p className="editor-tools-hint">Select a tool to edit, or add a new one.</p>
          )}

          <section className="editor-tools-test-panel" aria-label="Utterance test">
            <h3>Test panel</h3>
            <p className="editor-tools-hint">Type an utterance to preview target resolution (no mic).</p>
            <div className="editor-tools-test-row">
              <input
                type="text"
                className="editor-formula-input"
                value={testUtterance}
                onChange={(e) => setTestUtterance(e.target.value)}
                placeholder='e.g. "open notepad on the left"'
                aria-label="Test utterance"
              />
              <button type="button" className="editor-tools-inline-btn" onClick={() => void runPreview()}>
                Parse
              </button>
            </div>
            {previewError ? (
              <p className="editor-tools-preview-error" role="alert">
                {previewError}
              </p>
            ) : null}
            {previewLines ? (
              <dl className="editor-tools-preview">
                <div>
                  <dt>Target</dt>
                  <dd>{previewLines.targetLine}</dd>
                </div>
                <div>
                  <dt>Placement</dt>
                  <dd>{previewLines.placementLine}</dd>
                </div>
                <div>
                  <dt>Resolved</dt>
                  <dd>{previewLines.resolvedLine}</dd>
                </div>
              </dl>
            ) : null}
          </section>
        </div>
      </div>
    </div>
  );
}
