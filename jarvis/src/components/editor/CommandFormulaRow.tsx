import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ActionPayload, CommandNodePayload } from "../../types";
import { formatUserError } from "../../utils/userErrors";
import { useEditorStore } from "../../store/editorStore";
import { ACTION_KIND_OPTIONS, getActionKind } from "./actionCatalog";
import {
  deriveAppSearchMeta,
  deriveOpenAppDisplayMode,
  formulaArgInputClass,
  fingerprintCommandNode,
} from "./formulaRow.logic";
import {
  defaultActionForKind,
  hasBlockingErrors,
  modelFromNode,
  toCommandPayload,
  validateFormModel,
  type ActionKind,
  type FormModel,
} from "./NodeForm.logic";
import { searchAppIndexInvokeArgs } from "./appIndexInvoke";

export type AppIndexEntry = {
  display_name: string;
  exe_path: string;
  icon_data_url?: string | null;
};

type CommandFormulaRowProps = {
  node: CommandNodePayload;
  onToggleEnabled: () => void;
  onDelete: () => void;
  errorText?: string | null;
};

export function CommandFormulaRow({
  node,
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
      triggerPhrases: next.length > 0 ? [next] : [],
    }));
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
      <div className="editor-command-card">
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
            className="editor-formula-input editor-formula-input--phrase"
            value={primaryPhrase}
            onChange={(e) => setPrimaryPhrase(e.target.value)}
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
                      +
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
              className="editor-command-delete"
              onClick={onDelete}
              aria-label={`Delete ${primaryPhrase.trim() || "command"}`}
            >
              ×
            </button>
          </div>
        </div>

        {(errors.actions || errors.triggerPhrases || Object.keys(errors.actionErrors).length > 0) && (
          <p className="editor-field-error editor-command-row-errors">
            {[errors.triggerPhrases, errors.actions, ...Object.values(errors.actionErrors)]
              .filter(Boolean)
              .join(" ")}
          </p>
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
      triggerPhrases: next.length > 0 ? [next] : [],
    }));
  };

  const onSave = async () => {
    const errors = validateFormModel(model);
    if (hasBlockingErrors(errors)) {
      const actionErr = Object.values(errors.actionErrors)[0];
      setToastText(
        errors.triggerPhrases ?? errors.actions ?? actionErr ?? "Fix errors first.",
      );
      return;
    }
    setSaving(true);
    try {
      await invoke<CommandNodePayload>("create_command", {
        node: toCommandPayload(model),
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
      <div className="editor-command-card">
        {toastText && (
          <div className="editor-inline-toast editor-command-row-toast" role="alert">
            {toastText}
          </div>
        )}
        <div className="editor-command-formula">
          <input
            type="text"
            className="editor-formula-input editor-formula-input--phrase"
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
                    +
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

function AppIconImg({
  iconUrl,
  label,
  className,
}: {
  iconUrl: string | null | undefined;
  label: string;
  className: string;
}) {
  const [failed, setFailed] = useState(false);
  if (!iconUrl || failed) {
    return (
      <span className={`${className} ${className}--fallback`} aria-hidden>
        {label.trim().charAt(0).toUpperCase() || "A"}
      </span>
    );
  }
  return (
    <img
      src={iconUrl}
      alt=""
      className={className}
      loading="lazy"
      decoding="async"
      onError={() => setFailed(true)}
    />
  );
}

function ActionSegmentEditor({ action, index, onChange, onRemove, canRemove }: SegmentProps) {
  const kind = getActionKind(action);
  const [kindQuery, setKindQuery] = useState(
    () => ACTION_KIND_OPTIONS.find((opt) => opt.id === kind)?.label ?? kind,
  );
  const [kindOpen, setKindOpen] = useState(false);

  const [appQuery, setAppQuery] = useState(() => ("open_app" in action ? action.open_app.name : ""));
  const [appHits, setAppHits] = useState<AppIndexEntry[]>([]);
  const [appOpen, setAppOpen] = useState(false);
  const [appLoading, setAppLoading] = useState(false);
  const [appHasSearched, setAppHasSearched] = useState(false);
  const [appEditing, setAppEditing] = useState(
    () => !("open_app" in action) || action.open_app.path.trim().length === 0,
  );
  const [selectedAppIcon, setSelectedAppIcon] = useState<string | null>(null);
  const appTimer = useRef<number | null>(null);

  /* Local pickers mirror `action` / `kind` from the parent when the node reloads or the segment kind changes. */
  /* eslint-disable react-hooks/set-state-in-effect -- intentional props → local state sync */
  useEffect(() => {
    if ("open_app" in action) {
      setAppQuery(action.open_app.name);
      if (action.open_app.path.trim().length === 0) {
        setAppEditing(true);
      }
    } else {
      setAppEditing(true);
      setSelectedAppIcon(null);
    }
  }, [action]);

  useEffect(() => {
    setKindQuery(ACTION_KIND_OPTIONS.find((opt) => opt.id === kind)?.label ?? kind);
  }, [kind]);
  /* eslint-enable react-hooks/set-state-in-effect */

  useEffect(() => {
    if (!("open_app" in action) || !appOpen) return;
    if (appTimer.current) window.clearTimeout(appTimer.current);
    appTimer.current = window.setTimeout(() => {
      setAppLoading(true);
      void invoke<AppIndexEntry[]>("search_app_index", searchAppIndexInvokeArgs(appQuery, 24))
        .then((hits) => {
          setAppHits(hits);
          setAppHasSearched(true);
        })
        .catch(() => {
          setAppHits([]);
          setAppHasSearched(true);
        })
        .finally(() => setAppLoading(false));
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
      setAppHasSearched(false);
      setAppEditing(true);
      setSelectedAppIcon(null);
    }
  };

  const kindHits = useMemo(() => {
    const q = kindQuery.trim().toLowerCase();
    if (!q) return ACTION_KIND_OPTIONS;
    return ACTION_KIND_OPTIONS.filter(
      (opt) =>
        opt.label.toLowerCase().includes(q) ||
        opt.haystack.includes(q) ||
        opt.id.split("_").join(" ").includes(q),
    );
  }, [kindQuery]);

  const applyKindOption = (nextKind: ActionKind) => {
    onPickKind(nextKind);
    setKindQuery(ACTION_KIND_OPTIONS.find((opt) => opt.id === nextKind)?.label ?? nextKind);
    setKindOpen(false);
  };

  const appSearchMeta = deriveAppSearchMeta({
    isOpen: appOpen,
    query: appQuery,
    isLoading: appLoading,
    hasSearched: appHasSearched,
    hitCount: appHits.length,
  });
  const appDisplayMode =
    "open_app" in action
      ? deriveOpenAppDisplayMode({
          isEditing: appEditing,
          selectedPath: action.open_app.path,
        })
      : "edit";

  const renderArg = () => {
    if ("open_app" in action) {
      if (appDisplayMode === "confirmed") {
        return (
          <button
            type="button"
            className={`${formulaArgInputClass()} editor-formula-confirmed-chip`}
            title={action.open_app.name || "App"}
            onClick={() => {
              setAppEditing(true);
              setAppOpen(false);
              setAppHasSearched(false);
            }}
            aria-label={`Selected app ${action.open_app.name}. Click to change app.`}
          >
            <AppIconImg
              key={selectedAppIcon ?? `fallback:${action.open_app.path}`}
              iconUrl={selectedAppIcon}
              label={action.open_app.name || "App"}
              className="editor-formula-suggest-icon"
            />
          </button>
        );
      }
      return (
        <div className="editor-formula-arg-wrap">
          <input
            type="text"
            className={formulaArgInputClass()}
            value={appQuery}
            onChange={(e) => {
              setAppQuery(e.target.value);
              setAppHasSearched(false);
              setAppEditing(true);
              onChange({
                open_app: { name: e.target.value, path: "" },
              });
            }}
            onFocus={() => {
              setAppOpen(true);
              setAppLoading(true);
              void invoke<AppIndexEntry[]>("search_app_index", searchAppIndexInvokeArgs(appQuery, 24))
                .then((hits) => {
                  setAppHits(hits);
                  setAppHasSearched(true);
                })
                .catch(() => {
                  setAppHits([]);
                  setAppHasSearched(true);
                })
                .finally(() => setAppLoading(false));
            }}
            onBlur={() => window.setTimeout(() => setAppOpen(false), 120)}
            placeholder="Search app…"
            aria-label={`App name for step ${index + 1}`}
          />
          {appOpen && (
            <ul className="editor-formula-suggest" role="listbox">
              {appSearchMeta.countText && (
                <li role="none" className="editor-formula-suggest-meta">
                  {appSearchMeta.countText}
                </li>
              )}
              {appSearchMeta.statusText && (
                <li role="none" className="editor-formula-suggest-status">
                  {appSearchMeta.statusText}
                </li>
              )}
              {appHits.map((h) => (
                <li key={h.exe_path} role="none" className="editor-formula-suggest-li--icon">
                  <button
                    type="button"
                    role="option"
                    className="editor-formula-suggest-btn editor-formula-suggest-btn--icon-only"
                    title={h.display_name}
                    aria-label={`${h.display_name}, ${h.exe_path}`}
                    onMouseDown={(e) => e.preventDefault()}
                    onClick={() => {
                      onChange({ open_app: { name: h.display_name, path: h.exe_path } });
                      setAppQuery(h.display_name);
                      setAppHasSearched(false);
                      setSelectedAppIcon(h.icon_data_url ?? null);
                      setAppEditing(false);
                      setAppOpen(false);
                    }}
                  >
                    <span className="editor-formula-suggest-app editor-formula-suggest-app--icon-only">
                      <AppIconImg
                        key={`${h.exe_path}:${h.icon_data_url ?? ""}`}
                        iconUrl={h.icon_data_url}
                        label={h.display_name}
                        className="editor-formula-suggest-icon"
                      />
                      <span className="editor-formula-suggest-hover-label" aria-hidden="true">
                        {h.display_name}
                      </span>
                    </span>
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
          className={formulaArgInputClass()}
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
          className={formulaArgInputClass()}
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
          className={formulaArgInputClass()}
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
            className={formulaArgInputClass()}
            value={action.run_script.script}
            onChange={(e) =>
              onChange({ run_script: { ...action.run_script, script: e.target.value } })
            }
            placeholder="Script path"
            aria-label={`Script path for step ${index + 1}`}
          />
          <input
            type="text"
            className={formulaArgInputClass()}
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
          className={formulaArgInputClass()}
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
          className={formulaArgInputClass({ narrow: true, autoGrow: false })}
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
        <input
          type="text"
          className="editor-formula-input editor-formula-input--kind"
          value={kindQuery}
          onChange={(e) => {
            setKindQuery(e.target.value);
            setKindOpen(true);
          }}
          onFocus={() => setKindOpen(true)}
          onBlur={() => {
            window.setTimeout(() => {
              setKindOpen(false);
              setKindQuery(ACTION_KIND_OPTIONS.find((opt) => opt.id === kind)?.label ?? kind);
            }, 120);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === "Tab") {
              const pick = kindHits[0];
              if (!pick) return;
              e.preventDefault();
              applyKindOption(pick.id);
            }
          }}
          placeholder="Action"
          aria-label={`Action type for step ${index + 1}`}
        />
        {kindOpen && kindHits.length > 0 && (
          <ul className="editor-formula-suggest" role="listbox">
            {kindHits.map((opt) => (
              <li key={opt.id} role="none">
                <button
                  type="button"
                  role="option"
                  className="editor-formula-suggest-btn"
                  onMouseDown={(e) => e.preventDefault()}
                  onClick={() => applyKindOption(opt.id)}
                >
                  <span className="editor-formula-suggest-title">{opt.label}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
      <div className="editor-formula-arg-slot">{renderArg()}</div>
      {canRemove && (
        <button
          type="button"
          className="editor-formula-remove-inline"
          onClick={onRemove}
          aria-label={`Remove step ${index + 1}`}
        >
          ×
        </button>
      )}
    </div>
  );
}
