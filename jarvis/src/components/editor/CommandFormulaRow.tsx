import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ActionPayload, CommandNodePayload } from "../../types";
import { formatUserError } from "../../utils/userErrors";
import { useEditorStore } from "../../store/editorStore";
import { ACTION_KIND_OPTIONS, getActionKind } from "./actionCatalog";
import { fingerprintCommandNode } from "./formulaRow.logic";
import {
  defaultActionForKind,
  hasBlockingErrors,
  modelFromNode,
  toCommandPayload,
  validateFormModel,
  type ActionKind,
  type FormModel,
} from "./NodeForm.logic";

export type AppIndexEntry = {
  display_name: string;
  exe_path: string;
};

type CommandFormulaRowProps = {
  node: CommandNodePayload;
  expanded: boolean;
  onToggleExpand: () => void;
  onToggleEnabled: () => void;
  onDelete: () => void;
  errorText?: string | null;
};

export function CommandFormulaRow({
  node,
  expanded,
  onToggleExpand,
  onToggleEnabled,
  onDelete,
  errorText,
}: CommandFormulaRowProps) {
  const setNodes = useEditorStore((s) => s.setNodes);
  const [model, setModel] = useState<FormModel>(() => modelFromNode(node));
  const [toastText, setToastText] = useState<string | null>(null);
  const dirtyRef = useRef(false);
  const toastTimer = useRef<number | null>(null);

  const serverPrint = useMemo(() => fingerprintCommandNode(node), [node]);

  useEffect(() => {
    // Rehydrate local form when the server-backed node snapshot changes (e.g. list refresh).
    queueMicrotask(() => {
      setModel(modelFromNode(node));
      dirtyRef.current = false;
    });
  }, [serverPrint, node]);
  const modelRef = useRef(model);
  useEffect(() => {
    modelRef.current = model;
  }, [model]);

  useEffect(
    () => () => {
      if (toastTimer.current) window.clearTimeout(toastTimer.current);
    },
    [],
  );

  const showToast = useCallback((text: string) => {
    setToastText(text);
    if (toastTimer.current) window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => {
      setToastText(null);
      toastTimer.current = null;
    }, 2200);
  }, []);

  const updateModel = useCallback((updater: (prev: FormModel) => FormModel) => {
    dirtyRef.current = true;
    setModel((prev) => updater(prev));
  }, []);

  const payloadPrint = useMemo(() => JSON.stringify(toCommandPayload(model)), [model]);

  useEffect(() => {
    if (!model.id || !dirtyRef.current) return;
    const id = window.setTimeout(() => {
      const current = modelRef.current;
      if (!current.id) return;
      const errors = validateFormModel(current);
      if (hasBlockingErrors(errors)) return;
      const payload = toCommandPayload(current);
      void invoke<CommandNodePayload>("update_command", { id: current.id, node: payload })
        .then((saved) => {
          dirtyRef.current = false;
          const latest = useEditorStore.getState().nodes;
          setNodes(latest.map((n) => (n.id === saved.id ? saved : n)));
        })
        .catch((err: unknown) => {
          showToast(formatUserError(err, "Could not save."));
        });
    }, 520);
    return () => window.clearTimeout(id);
  }, [payloadPrint, model.id, setNodes, showToast]);

  const primaryPhrase = model.triggerPhrases[0] ?? "";
  const setPrimaryPhrase = (next: string) => {
    updateModel((prev) => ({
      ...prev,
      triggerPhrases: next.trim().length > 0 ? [next.trim()] : [],
    }));
  };

  const syncNameFromPhrase = () => {
    if (model.name.trim().length > 0) return;
    const p = (model.triggerPhrases[0] ?? "").trim();
    if (!p) return;
    updateModel((prev) => ({ ...prev, name: p.slice(0, 72) }));
  };

  const addActionSegment = () => {
    updateModel((prev) => ({
      ...prev,
      actions: [...prev.actions, defaultActionForKind("wait")],
    }));
  };

  const setActionAt = (index: number, next: ActionPayload) => {
    updateModel((prev) => {
      const actions = [...prev.actions];
      actions[index] = next;
      return { ...prev, actions };
    });
  };

  const removeActionAt = (index: number) => {
    updateModel((prev) => ({
      ...prev,
      actions: prev.actions.filter((_, i) => i !== index),
    }));
  };

  const errors = validateFormModel(model);

  return (
    <li className="editor-command-item">
      <div className={`editor-command-card${expanded ? " is-expanded" : ""}`}>
        {toastText && (
          <div className="editor-inline-toast editor-command-row-toast" role="status">
            {toastText}
          </div>
        )}
        {errorText && (
          <div className="editor-inline-toast" role="alert">
            {errorText}
          </div>
        )}

        <div className="editor-command-formula">
          <input
            type="text"
            className="editor-formula-input"
            value={primaryPhrase}
            onChange={(e) => setPrimaryPhrase(e.target.value)}
            onBlur={() => syncNameFromPhrase()}
            placeholder="Trigger phrase"
            aria-label="Trigger phrase"
          />
          <span className="editor-formula-eq" aria-hidden>
            =
          </span>

          <div className="editor-formula-chain" role="group" aria-label="Actions">
            {model.actions.length === 0 ? (
              <span className="editor-formula-muted">No actions</span>
            ) : (
              model.actions.map((action, index) => (
                <div key={`seg-${index}`} className="editor-formula-segment-wrap">
                  {index > 0 && (
                    <span className="editor-formula-arrow" aria-hidden>
                      →
                    </span>
                  )}
                  <ActionSegmentEditor
                    key={`${model.id ?? "draft"}-${index}-${getActionKind(action)}`}
                    action={action}
                    index={index}
                    onChange={(next) => setActionAt(index, next)}
                    onRemove={() => removeActionAt(index)}
                    canRemove={model.actions.length > 1}
                  />
                </div>
              ))
            )}
            <button
              type="button"
              className="editor-formula-plus"
              onClick={addActionSegment}
              aria-label="Add action"
            >
              +
            </button>
          </div>

          <div className="editor-command-trail">
            <button
              type="button"
              className={`editor-switch${model.enabled ? " is-on" : ""}`}
              role="switch"
              aria-checked={model.enabled}
              onClick={onToggleEnabled}
            >
              <span className="editor-switch-knob" />
            </button>
            <button
              type="button"
              className="editor-expand-chevron"
              aria-expanded={expanded}
              onClick={onToggleExpand}
              aria-label={expanded ? "Hide details" : "Show details"}
            >
              {expanded ? "⌄" : "›"}
            </button>
            <button
              type="button"
              className="editor-command-delete"
              onClick={onDelete}
              aria-label={`Delete ${model.name.trim() || primaryPhrase.trim() || "command"}`}
            >
              ×
            </button>
          </div>
        </div>

        {expanded && (
          <div className="editor-command-advanced">
            <div className="editor-form-grid">
              <label>
                Display name
                <input
                  value={model.name}
                  onChange={(e) => updateModel((p) => ({ ...p, name: e.target.value }))}
                  placeholder="Shown in lists"
                />
              </label>
              {model.triggerPhrases.length > 1 && (
                <p className="editor-settings-help">
                  Multiple trigger phrases are stored; the formula line edits the first phrase only.
                </p>
              )}
              <label>
                Fuzzy threshold: {model.threshold.toFixed(2)}
                <input
                  type="range"
                  min={0.5}
                  max={1}
                  step={0.01}
                  value={model.threshold}
                  onChange={(e) =>
                    updateModel((p) => ({ ...p, threshold: Number(e.target.value) }))
                  }
                />
              </label>
            </div>
            {(errors.actions || errors.triggerPhrases || errors.name || Object.keys(errors.actionErrors).length > 0) && (
              <p className="editor-field-error">
                {[errors.name, errors.triggerPhrases, errors.actions, ...Object.values(errors.actionErrors)]
                  .filter(Boolean)
                  .join(" ")}
              </p>
            )}
          </div>
        )}
      </div>
    </li>
  );
}

type DraftRowProps = {
  onDiscard: () => void;
  onCreated: () => void;
};

export function CommandDraftRow({ onDiscard, onCreated }: DraftRowProps) {
  const [model, setModel] = useState<FormModel>(() => ({
    ...modelFromNode(null),
    name: "",
    triggerPhrases: [],
    actions: [defaultActionForKind("open_app")],
  }));
  const [saving, setSaving] = useState(false);
  const [toastText, setToastText] = useState<string | null>(null);

  const updateModel = (updater: (prev: FormModel) => FormModel) => setModel((p) => updater(p));

  const primaryPhrase = model.triggerPhrases[0] ?? "";
  const setPrimaryPhrase = (next: string) => {
    updateModel((prev) => ({
      ...prev,
      triggerPhrases: next.trim().length > 0 ? [next.trim()] : [],
    }));
  };

  const onSave = async () => {
    let m = model;
    if (!m.name.trim() && (m.triggerPhrases[0] ?? "").trim()) {
      m = { ...m, name: (m.triggerPhrases[0] ?? "").trim().slice(0, 72) };
      setModel(m);
    }
    const errors = validateFormModel(m);
    if (hasBlockingErrors(errors)) {
      const actionErr = Object.values(errors.actionErrors)[0];
      setToastText(
        errors.name ?? errors.triggerPhrases ?? errors.actions ?? actionErr ?? "Fix errors first.",
      );
      return;
    }
    setSaving(true);
    try {
      await invoke<CommandNodePayload>("create_command", {
        node: toCommandPayload(m),
      });
      onCreated();
    } catch (err: unknown) {
      setToastText(formatUserError(err, "Could not create command."));
    } finally {
      setSaving(false);
    }
  };

  const addActionSegment = () =>
    updateModel((prev) => ({ ...prev, actions: [...prev.actions, defaultActionForKind("wait")] }));

  const setActionAt = (index: number, next: ActionPayload) => {
    updateModel((prev) => {
      const actions = [...prev.actions];
      actions[index] = next;
      return { ...prev, actions };
    });
  };

  const removeActionAt = (index: number) => {
    updateModel((prev) => ({
      ...prev,
      actions: prev.actions.filter((_, i) => i !== index),
    }));
  };

  return (
    <li className="editor-command-item editor-command-item--draft">
      <div className="editor-command-card is-expanded">
        {toastText && (
          <div className="editor-inline-toast editor-command-row-toast" role="alert">
            {toastText}
          </div>
        )}
        <div className="editor-command-formula">
          <input
            type="text"
            className="editor-formula-input"
            value={primaryPhrase}
            onChange={(e) => setPrimaryPhrase(e.target.value)}
            placeholder="New trigger phrase"
            aria-label="Trigger phrase"
          />
          <span className="editor-formula-eq" aria-hidden>
            =
          </span>
          <div className="editor-formula-chain" role="group" aria-label="Actions">
            {model.actions.map((action, index) => (
              <div key={`d-${index}`} className="editor-formula-segment-wrap">
                {index > 0 && (
                  <span className="editor-formula-arrow" aria-hidden>
                    →
                  </span>
                )}
                <ActionSegmentEditor
                  key={`draft-${index}-${getActionKind(action)}`}
                  action={action}
                  index={index}
                  onChange={(next) => setActionAt(index, next)}
                  onRemove={() => removeActionAt(index)}
                  canRemove={model.actions.length > 1}
                />
              </div>
            ))}
            <button type="button" className="editor-formula-plus" onClick={addActionSegment} aria-label="Add action">
              +
            </button>
          </div>
          <div className="editor-command-draft-actions">
            <button type="button" className="editor-settings-secondary-btn" onClick={onDiscard}>
              Cancel
            </button>
            <button type="button" onClick={() => void onSave()} disabled={saving}>
              {saving ? "Creating…" : "Create command"}
            </button>
          </div>
        </div>
      </div>
    </li>
  );
}

type SegmentProps = {
  action: ActionPayload;
  index: number;
  onChange: (next: ActionPayload) => void;
  onRemove: () => void;
  canRemove: boolean;
};

function ActionSegmentEditor({ action, index, onChange, onRemove, canRemove }: SegmentProps) {
  const kind = getActionKind(action);

  const [appQuery, setAppQuery] = useState(() => ("open_app" in action ? action.open_app.name : ""));
  const [appHits, setAppHits] = useState<AppIndexEntry[]>([]);
  const [appOpen, setAppOpen] = useState(false);
  const appTimer = useRef<number | null>(null);

  useEffect(() => {
    if ("open_app" in action) {
      setAppQuery(action.open_app.name);
    }
  }, [action]);

  useEffect(() => {
    if (!("open_app" in action) || !appOpen) return;
    if (appTimer.current) window.clearTimeout(appTimer.current);
    appTimer.current = window.setTimeout(() => {
      void invoke<AppIndexEntry[]>("search_app_index", { query: appQuery, limit: 24 })
        .then(setAppHits)
        .catch(() => setAppHits([]));
    }, 160);
    return () => {
      if (appTimer.current) window.clearTimeout(appTimer.current);
    };
  }, [appQuery, appOpen, action]);

  const onPickKind = (nextKind: ActionKind) => {
    onChange(defaultActionForKind(nextKind));
    if (nextKind === "open_app") {
      setAppQuery("");
      setAppOpen(true);
    }
  };

  const renderArg = () => {
    if ("open_app" in action) {
      return (
        <div className="editor-formula-arg-wrap">
          <input
            type="text"
            className="editor-formula-input editor-formula-input--arg"
            value={appQuery}
            onChange={(e) => {
              setAppQuery(e.target.value);
              onChange({
                open_app: { name: e.target.value, path: "" },
              });
            }}
            onFocus={() => {
              setAppOpen(true);
              void invoke<AppIndexEntry[]>("search_app_index", { query: appQuery, limit: 24 })
                .then(setAppHits)
                .catch(() => setAppHits([]));
            }}
            onBlur={() => window.setTimeout(() => setAppOpen(false), 120)}
            placeholder="Search app…"
            aria-label={`App name for step ${index + 1}`}
          />
          {appOpen && appHits.length > 0 && (
            <ul className="editor-formula-suggest" role="listbox">
              {appHits.map((h) => (
                <li key={h.exe_path} role="none">
                  <button
                    type="button"
                    role="option"
                    className="editor-formula-suggest-btn"
                    onMouseDown={(e) => e.preventDefault()}
                    onClick={() => {
                      onChange({ open_app: { name: h.display_name, path: h.exe_path } });
                      setAppQuery(h.display_name);
                      setAppOpen(false);
                    }}
                  >
                    <span className="editor-formula-suggest-title">{h.display_name}</span>
                    <span className="editor-formula-suggest-sub">{h.exe_path}</span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      );
    }
    if ("open_url" in action) {
      return (
        <input
          type="url"
          className="editor-formula-input editor-formula-input--arg"
          value={action.open_url.url}
          onChange={(e) => onChange({ open_url: { url: e.target.value } })}
          placeholder="https://…"
          aria-label={`URL for step ${index + 1}`}
        />
      );
    }
    if ("speak" in action) {
      return (
        <input
          type="text"
          className="editor-formula-input editor-formula-input--arg"
          value={action.speak.text}
          onChange={(e) => onChange({ speak: { text: e.target.value } })}
          placeholder="Words to speak"
          aria-label={`Speak text for step ${index + 1}`}
        />
      );
    }
    if ("send_keys" in action) {
      return (
        <input
          type="text"
          className="editor-formula-input editor-formula-input--arg"
          value={action.send_keys.keys}
          onChange={(e) => onChange({ send_keys: { keys: e.target.value } })}
          placeholder="ctrl+shift+p"
          aria-label={`Keys for step ${index + 1}`}
        />
      );
    }
    if ("run_script" in action) {
      return (
        <div className="editor-formula-arg-wrap editor-formula-arg-wrap--stack">
          <input
            type="text"
            className="editor-formula-input editor-formula-input--arg"
            value={action.run_script.script}
            onChange={(e) =>
              onChange({ run_script: { ...action.run_script, script: e.target.value } })
            }
            placeholder="Script path"
            aria-label={`Script path for step ${index + 1}`}
          />
          <input
            type="text"
            className="editor-formula-input editor-formula-input--arg"
            value={action.run_script.args.join(", ")}
            onChange={(e) =>
              onChange({
                run_script: {
                  ...action.run_script,
                  args: e.target.value
                    .split(",")
                    .map((part) => part.trim())
                    .filter((part) => part.length > 0),
                },
              })
            }
            placeholder="Arguments (comma-separated)"
            aria-label={`Script arguments for step ${index + 1}`}
          />
        </div>
      );
    }
    if ("sub_prompt" in action) {
      return (
        <input
          type="text"
          className="editor-formula-input editor-formula-input--arg"
          value={action.sub_prompt.prompt}
          onChange={(e) => onChange({ sub_prompt: { prompt: e.target.value } })}
          placeholder="Follow-up question"
          aria-label={`Sub-prompt text for step ${index + 1}`}
        />
      );
    }
    if ("wait" in action) {
      return (
        <input
          type="number"
          className="editor-formula-input editor-formula-input--arg editor-formula-input--narrow"
          min={0}
          value={action.wait.ms}
          onChange={(e) =>
            onChange({
              wait: {
                ms: Number.isFinite(Number(e.target.value)) ? Math.max(0, Number(e.target.value)) : 0,
              },
            })
          }
          aria-label={`Wait milliseconds for step ${index + 1}`}
        />
      );
    }
    return null;
  };

  return (
    <div className="editor-formula-segment">
      <div className="editor-formula-kind-wrap">
        <select
          className="editor-formula-input"
          value={kind}
          onChange={(e) => onPickKind(e.target.value as ActionKind)}
          aria-label={`Action type for step ${index + 1}`}
        >
          {ACTION_KIND_OPTIONS.map((opt) => (
            <option key={opt.id} value={opt.id}>
              {opt.label}
            </option>
          ))}
        </select>
      </div>
      <div className="editor-formula-arg-slot">{renderArg()}</div>
      {canRemove && (
        <button
          type="button"
          className="editor-formula-remove"
          onClick={onRemove}
          aria-label={`Remove step ${index + 1}`}
        >
          −
        </button>
      )}
    </div>
  );
}
