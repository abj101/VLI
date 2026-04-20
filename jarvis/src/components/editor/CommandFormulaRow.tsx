import { invoke } from "@tauri-apps/api/core";
import { createPortal } from "react-dom";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type FocusEvent,
  type ReactNode,
  type RefObject,
} from "react";
import type { CommandNodePayload, FormActionPayload } from "../../types";
import { editorPendingAction } from "../../types";
import { formatUserError } from "../../utils/userErrors";
import { useEditorStore } from "../../store/editorStore";
import { useSettingsStore } from "../../store/settingsStore";
import { ACTION_KIND_OPTIONS, getActionKind } from "./actionCatalog";
import {
  appExeDisplayLabel,
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
  type ConcreteActionKind,
  type FormModel,
} from "./NodeForm.logic";
import { searchAppIndexInvokeArgs } from "./appIndexInvoke";
import { EditorCloseXIcon } from "./EditorCloseXIcon";

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

type FixedSuggestPos = { top: number; left: number; width: number; maxHeight: number };

/** Renders formula autocomplete under anchor; portals to `body` so parent `overflow` on command list does not clip. Mount only while open so layout state resets without effect setState on close. */
function FormulaSuggestPortal({
  anchorRef,
  children,
}: {
  anchorRef: RefObject<HTMLElement | null>;
  children: ReactNode;
}) {
  const [pos, setPos] = useState<FixedSuggestPos | null>(null);

  const sync = useCallback(() => {
    const el = anchorRef.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const margin = 8;
    const gap = 4;
    const top = r.bottom + gap;
    const maxHeight = Math.max(96, Math.min(280, window.innerHeight - top - margin));
    const width = Math.min(r.width, window.innerWidth - margin * 2);
    const left = Math.min(Math.max(margin, r.left), window.innerWidth - margin - width);
    setPos({ top, left, width, maxHeight });
  }, [anchorRef]);

  useLayoutEffect(() => {
    sync();
    const el = anchorRef.current;
    window.addEventListener("resize", sync);
    window.addEventListener("scroll", sync, true);
    const ro = el ? new ResizeObserver(() => queueMicrotask(sync)) : null;
    if (el && ro) ro.observe(el);
    return () => {
      window.removeEventListener("resize", sync);
      window.removeEventListener("scroll", sync, true);
      ro?.disconnect();
    };
  }, [sync, anchorRef]);

  if (!pos) return null;

  return createPortal(
    <ul
      className="editor-formula-suggest editor-formula-suggest--portal"
      role="listbox"
      style={{
        top: pos.top,
        left: pos.left,
        width: pos.width,
        maxHeight: pos.maxHeight,
      }}
    >
      {children}
    </ul>,
    document.body,
  );
}

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
      actions: [...prev.actions, editorPendingAction()],
    }));
  };

  const setActionAt = (index: number, next: FormActionPayload) => {
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
  const followUpVariableMeta = useMemo(() => deriveFollowUpVariableMap(model.actions), [model.actions]);

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
                    availableVariableLabels={followUpVariableMeta.labels}
                    variableLabel={
                      followUpVariableMeta.byActionIndex.get(index)
                        ? `Variable ${followUpVariableMeta.byActionIndex.get(index)}`
                        : undefined
                    }
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
              <EditorCloseXIcon className="editor-command-delete-x" />
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

function DraftCheckIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" aria-hidden className="editor-command-draft-icon-svg">
      <path
        d="M6 12.5L10.2 16.5L18 7.5"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function DraftBusyIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" aria-hidden className="editor-command-draft-icon-svg">
      <circle cx="7" cy="12" r="1.75" fill="currentColor" />
      <circle cx="12" cy="12" r="1.75" fill="currentColor" />
      <circle cx="17" cy="12" r="1.75" fill="currentColor" />
    </svg>
  );
}

export function CommandDraftRow({ onDiscard, onCreated }: DraftRowProps) {
  const [model, setModel] = useState<FormModel>(() => ({
    ...modelFromNode(null),
    triggerPhrases: [],
    actions: [editorPendingAction()],
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
    updateModel((prev) => ({ ...prev, actions: [...prev.actions, editorPendingAction()] }));

  const setActionAt = (index: number, next: FormActionPayload) => {
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
  const followUpVariableMeta = useMemo(() => deriveFollowUpVariableMap(model.actions), [model.actions]);

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
                  availableVariableLabels={followUpVariableMeta.labels}
                  variableLabel={
                    followUpVariableMeta.byActionIndex.get(index)
                      ? `Variable ${followUpVariableMeta.byActionIndex.get(index)}`
                      : undefined
                  }
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
            <button
              type="button"
              className="editor-command-draft-icon-btn"
              onClick={onDiscard}
              aria-label="Cancel"
            >
              <span className="editor-command-draft-icon" aria-hidden>
                <EditorCloseXIcon className="editor-command-draft-icon-svg" />
              </span>
            </button>
            <button
              type="button"
              className="editor-command-draft-icon-btn editor-command-draft-icon-btn--accent"
              onClick={() => void onSave()}
              disabled={saving}
              aria-label={saving ? "Saving…" : "Save"}
            >
              <span className="editor-command-draft-icon" aria-hidden>
                {saving ? <DraftBusyIcon /> : <DraftCheckIcon />}
              </span>
            </button>
          </div>
        </div>
      </div>
    </li>
  );
}

type SegmentProps = {
  action: FormActionPayload;
  index: number;
  availableVariableLabels: string[];
  variableLabel?: string;
  onChange: (next: FormActionPayload) => void;
  onRemove: () => void;
  canRemove: boolean;
};

type VariableTokenContext = {
  start: number;
  end: number;
  query: string;
};

export function deriveFollowUpVariableMap(actions: FormActionPayload[]) {
  let next = 0;
  const byActionIndex = new Map<number, number>();
  actions.forEach((action, idx) => {
    if (!("sub_prompt" in action)) return;
    next += 1;
    byActionIndex.set(idx, next);
  });
  return {
    byActionIndex,
    labels: Array.from({ length: next }, (_, i) => `Variable ${i + 1}`),
  };
}

export function extractVariableTokenContext(inputValue: string, caret: number): VariableTokenContext | null {
  const before = inputValue.slice(0, caret);
  const match = /(^|\s)(Variable(?:\s+\d*)?)$/i.exec(before);
  if (!match) return null;
  const token = match[2];
  return {
    start: before.length - token.length,
    end: caret,
    query: token.replace(/^Variable/i, "").trim(),
  };
}

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

function ActionSegmentEditor({
  action,
  index,
  availableVariableLabels,
  variableLabel,
  onChange,
  onRemove,
  canRemove,
}: SegmentProps) {
  const kindAnchorRef = useRef<HTMLDivElement>(null);
  const appAnchorRef = useRef<HTMLDivElement>(null);
  const variableAnchorRef = useRef<HTMLDivElement>(null);
  const variableTargetRef = useRef<{
    value: string;
    start: number;
    end: number;
    onCommit: (next: string) => void;
    input: HTMLInputElement;
  } | null>(null);
  const kind = getActionKind(action);
  const [kindQuery, setKindQuery] = useState(() =>
    kind === "pending" ? "" : (ACTION_KIND_OPTIONS.find((opt) => opt.id === kind)?.label ?? kind),
  );
  const [kindOpen, setKindOpen] = useState(false);

  const [appQuery, setAppQuery] = useState(() => ("open_app" in action ? action.open_app.name : ""));
  const [appHits, setAppHits] = useState<AppIndexEntry[]>([]);
  /** Lazy-loaded icons for current dropdown rows (`null` = fetched, no icon). */
  const [appHitIcons, setAppHitIcons] = useState<Record<string, string | null>>({});
  const [appOpen, setAppOpen] = useState(false);
  const [appHasSearched, setAppHasSearched] = useState(false);
  const [variableOpen, setVariableOpen] = useState(false);
  const [variableQuery, setVariableQuery] = useState("");
  const [appEditing, setAppEditing] = useState(
    () => !("open_app" in action) || action.open_app.path.trim().length === 0,
  );
  const [selectedAppIcon, setSelectedAppIcon] = useState<string | null>(null);
  const appTimer = useRef<number | null>(null);

  useEffect(() => {
    if (!appOpen) {
      setAppHitIcons({});
      return;
    }
    if (!appHits.length) return;

    let cancelled = false;
    const targets = appHits.filter((h) => {
      if (h.icon_data_url) return false;
      const p = h.exe_path.trim();
      if (!p) return false;
      const low = p.toLowerCase();
      if (low.startsWith("shell:")) return false;
      if (p.includes("://")) return false;
      return true;
    });

    const run = async () => {
      const CONCURRENCY = 4;
      for (let i = 0; i < targets.length && !cancelled; i += CONCURRENCY) {
        const slice = targets.slice(i, i + CONCURRENCY);
        await Promise.all(
          slice.map(async (h) => {
            if (cancelled) return;
            try {
              const icon = await invoke<string | null>("get_app_icon", {
                payload: { path: h.exe_path },
              });
              if (!cancelled) {
                setAppHitIcons((prev) => {
                  if (Object.prototype.hasOwnProperty.call(prev, h.exe_path)) return prev;
                  return { ...prev, [h.exe_path]: icon };
                });
              }
            } catch {
              if (!cancelled) {
                setAppHitIcons((prev) => {
                  if (Object.prototype.hasOwnProperty.call(prev, h.exe_path)) return prev;
                  return { ...prev, [h.exe_path]: null };
                });
              }
            }
          }),
        );
      }
    };
    void run();
    return () => {
      cancelled = true;
    };
  }, [appOpen, appHits]);

  /* Local pickers mirror `action` / `kind` from the parent when the node reloads or the segment kind changes. */
  useEffect(() => {
    if ("editor_pending" in action) {
      return;
    }
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
    setKindQuery(
      kind === "pending" ? "" : (ACTION_KIND_OPTIONS.find((opt) => opt.id === kind)?.label ?? kind),
    );
  }, [kind]);

  // Lazy icon fetch for already-saved open_app actions (scanner no longer
  // ships icons inline, so they need to be pulled on first render).
  const selectedAppPath = "open_app" in action ? action.open_app.path : "";
  useEffect(() => {
    if (!selectedAppPath || selectedAppPath.startsWith("shell:") || selectedAppPath.includes("://")) {
      return;
    }
    if (selectedAppIcon) return;
    let cancelled = false;
    void invoke<string | null>("get_app_icon", { payload: { path: selectedAppPath } })
      .then((icon) => {
        if (!cancelled) setSelectedAppIcon(icon ?? null);
      })
      .catch(() => {
        /* fall back to letter */
      });
    return () => {
      cancelled = true;
    };
    // Only refetch when the underlying path changes; we intentionally skip
    // `selectedAppIcon` as a dep so a null result doesn't retrigger.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedAppPath]);

  useEffect(() => {
    if (!("open_app" in action) || !appOpen) return;
    if (appTimer.current) window.clearTimeout(appTimer.current);
    appTimer.current = window.setTimeout(() => {
      void invoke<AppIndexEntry[]>("search_app_index", searchAppIndexInvokeArgs(appQuery, 120))
        .then((hits) => {
          setAppHits(hits);
          setAppHasSearched(true);
        })
        .catch(() => {
          setAppHits([]);
          setAppHasSearched(true);
        });
    }, 200);
    return () => {
      if (appTimer.current) window.clearTimeout(appTimer.current);
    };
  }, [appQuery, appOpen, action]);

  const onPickKind = (nextKind: ConcreteActionKind) => {
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

  const applyKindOption = (nextKind: ConcreteActionKind) => {
    onPickKind(nextKind);
    setKindQuery(ACTION_KIND_OPTIONS.find((opt) => opt.id === nextKind)?.label ?? nextKind);
    setKindOpen(false);
  };

  const appIndexCount = useSettingsStore((s) => s.appIndexCount);
  const appIndexScanning = useSettingsStore((s) => s.appIndexScanning);
  const appSearchMeta = deriveAppSearchMeta({
    isOpen: appOpen,
    query: appQuery,
    isLoading: false,
    hasSearched: appHasSearched,
    hitCount: appHits.length,
    indexCount: appIndexCount,
    isScanning: appIndexScanning,
  });
  const appDisplayMode =
    "open_app" in action
      ? deriveOpenAppDisplayMode({
          isEditing: appEditing,
          selectedPath: action.open_app.path,
        })
      : "edit";
  const variableHits = useMemo(() => {
    if (!availableVariableLabels.length) return [];
    const q = variableQuery.trim().toLowerCase();
    if (!q) return availableVariableLabels;
    return availableVariableLabels.filter((label) => label.toLowerCase().startsWith(`variable ${q}`));
  }, [availableVariableLabels, variableQuery]);

  const updateVariableSuggest = useCallback(
    (input: HTMLInputElement, nextValue: string, onCommit: (next: string) => void) => {
      if (!availableVariableLabels.length) {
        setVariableOpen(false);
        return;
      }
      const caret = input.selectionStart ?? nextValue.length;
      const token = extractVariableTokenContext(nextValue, caret);
      if (!token) {
        setVariableOpen(false);
        return;
      }
      variableTargetRef.current = {
        value: nextValue,
        start: token.start,
        end: token.end,
        onCommit,
        input,
      };
      setVariableQuery(token.query);
      setVariableOpen(true);
    },
    [availableVariableLabels],
  );

  const bindVariableSuggestInput = useCallback(
    (nextValue: string, onCommit: (next: string) => void) => ({
      onChange: (e: ChangeEvent<HTMLInputElement>) => {
        const value = e.target.value;
        onCommit(value);
        updateVariableSuggest(e.currentTarget, value, onCommit);
      },
      onFocus: (e: FocusEvent<HTMLInputElement>) => {
        updateVariableSuggest(e.currentTarget, nextValue, onCommit);
      },
      onBlur: () => {
        window.setTimeout(() => setVariableOpen(false), 120);
      },
    }),
    [updateVariableSuggest],
  );

  const applyVariableOption = useCallback((label: string) => {
    const target = variableTargetRef.current;
    if (!target) return;
    const next = `${target.value.slice(0, target.start)}${label}${target.value.slice(target.end)}`;
    target.onCommit(next);
    setVariableOpen(false);
    window.setTimeout(() => {
      try {
        const nextPos = target.start + label.length;
        target.input.focus();
        target.input.setSelectionRange(nextPos, nextPos);
      } catch {
        // no-op: blur during async replace
      }
    }, 0);
  }, []);

  const renderArg = () => {
    if ("editor_pending" in action) {
      return null;
    }
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
      const showAppLeadingIcon = action.open_app.path.trim().length > 0;
      return (
        <div
          className={
            showAppLeadingIcon
              ? "editor-formula-arg-wrap editor-formula-arg-wrap--leading-app-icon"
              : "editor-formula-arg-wrap"
          }
          ref={appAnchorRef}
        >
          {showAppLeadingIcon ? (
            <span className="editor-formula-input-leading-icon" aria-hidden>
              <AppIconImg
                key={selectedAppIcon ?? `path:${action.open_app.path}`}
                iconUrl={selectedAppIcon}
                label={action.open_app.name || "App"}
                className="editor-formula-suggest-icon"
              />
            </span>
          ) : null}
          <input
            type="text"
            className={formulaArgInputClass()}
            value={appQuery}
            onChange={(e) => {
              const v = e.target.value;
              setAppQuery(v);
              setAppEditing(true);
              onChange({
                open_app: { name: v, path: "" },
              });
            }}
            onFocus={() => {
              setAppOpen(true);
            }}
            onBlur={() => window.setTimeout(() => setAppOpen(false), 120)}
            placeholder="Search app…"
            aria-label={`App name for step ${index + 1}`}
          />
          {appOpen ? (
            <FormulaSuggestPortal anchorRef={appAnchorRef}>
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
              {appHits.map((h) => {
                const rowIcon = h.icon_data_url ?? appHitIcons[h.exe_path] ?? undefined;
                const fileLabel = appExeDisplayLabel(h.exe_path);
                return (
                  <li key={h.exe_path} role="none">
                    <button
                      type="button"
                      role="option"
                      className="editor-formula-suggest-btn"
                      title={`${h.display_name}\n${h.exe_path}`}
                      aria-label={`${h.display_name}, ${h.exe_path}`}
                      onMouseDown={(e) => e.preventDefault()}
                      onClick={() => {
                        onChange({ open_app: { name: h.display_name, path: h.exe_path } });
                        setAppQuery(h.display_name);
                        setAppHasSearched(false);
                        const picked = h.icon_data_url ?? appHitIcons[h.exe_path] ?? null;
                        setSelectedAppIcon(picked);
                        setAppEditing(false);
                        setAppOpen(false);
                        if (!picked && h.exe_path) {
                          void invoke<string | null>("get_app_icon", {
                            payload: { path: h.exe_path },
                          })
                            .then((icon) => setSelectedAppIcon(icon ?? null))
                            .catch(() => setSelectedAppIcon(null));
                        }
                      }}
                    >
                      <span className="editor-formula-suggest-app">
                        <AppIconImg
                          key={`${h.exe_path}:${rowIcon ?? ""}`}
                          iconUrl={rowIcon}
                          label={h.display_name}
                          className="editor-formula-suggest-icon"
                        />
                        <span className="editor-formula-suggest-text">
                          <span className="editor-formula-suggest-title">{h.display_name}</span>
                          <span className="editor-formula-suggest-sub">{fileLabel}</span>
                        </span>
                      </span>
                    </button>
                  </li>
                );
              })}
            </FormulaSuggestPortal>
          ) : null}
        </div>
      );
    }
    if ("open_url" in action) {
      return (
        <div className="editor-formula-arg-wrap" ref={variableAnchorRef}>
          <input
            type="url"
            className={formulaArgInputClass()}
            value={action.open_url.url}
            {...bindVariableSuggestInput(action.open_url.url, (value) => onChange({ open_url: { url: value } }))}
            placeholder="https://…"
            aria-label={`URL for step ${index + 1}`}
          />
        </div>
      );
    }
    if ("speak" in action) {
      return (
        <div className="editor-formula-arg-wrap" ref={variableAnchorRef}>
          <input
            type="text"
            className={formulaArgInputClass()}
            value={action.speak.text}
            {...bindVariableSuggestInput(action.speak.text, (value) => onChange({ speak: { text: value } }))}
            placeholder="Words to speak"
            aria-label={`Speak text for step ${index + 1}`}
          />
        </div>
      );
    }
    if ("send_keys" in action) {
      return (
        <div className="editor-formula-arg-wrap" ref={variableAnchorRef}>
          <input
            type="text"
            className={formulaArgInputClass()}
            value={action.send_keys.keys}
            {...bindVariableSuggestInput(action.send_keys.keys, (value) => onChange({ send_keys: { keys: value } }))}
            placeholder="ctrl+shift+p"
            aria-label={`Keys for step ${index + 1}`}
          />
        </div>
      );
    }
    if ("run_script" in action) {
      return (
        <div className="editor-formula-arg-wrap editor-formula-arg-wrap--stack" ref={variableAnchorRef}>
          <input
            type="text"
            className={formulaArgInputClass()}
            value={action.run_script.script}
            {...bindVariableSuggestInput(action.run_script.script, (value) =>
              onChange({ run_script: { ...action.run_script, script: value } }),
            )}
            placeholder="Script path"
            aria-label={`Script path for step ${index + 1}`}
          />
          <input
            type="text"
            className={formulaArgInputClass()}
            value={action.run_script.args.join(", ")}
            {...bindVariableSuggestInput(action.run_script.args.join(", "), (value) =>
              onChange({
                run_script: {
                  ...action.run_script,
                  args: value
                    .split(",")
                    .map((part) => part.trim())
                    .filter((part) => part.length > 0),
                },
              })
            )}
            placeholder="Arguments (comma-separated)"
            aria-label={`Script arguments for step ${index + 1}`}
          />
        </div>
      );
    }
    if ("sub_prompt" in action) {
      return (
        <div className="editor-formula-arg-wrap" ref={variableAnchorRef}>
          <input
            type="text"
            className={formulaArgInputClass()}
            value={action.sub_prompt.prompt}
            {...bindVariableSuggestInput(action.sub_prompt.prompt, (value) =>
              onChange({ sub_prompt: { prompt: value } }),
            )}
            placeholder="Follow-up question"
            aria-label={`Follow-up text for step ${index + 1}`}
          />
        </div>
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

  const removeButton = canRemove ? (
    <button
      type="button"
      className="editor-formula-remove-inline"
      onClick={onRemove}
      aria-label={`Remove step ${index + 1}`}
    >
      <EditorCloseXIcon className="editor-formula-remove-inline-x" />
    </button>
  ) : null;

  const isPending = "editor_pending" in action;
  /** Pending rows have no arg field — inset remove belongs on the kind ("Action") control. */
  const removeInKind = canRemove && isPending;
  const removeInArg = canRemove && !isPending;

  const kindWrapClass = removeInKind
    ? "editor-formula-kind-wrap editor-formula-kind-wrap--clearable"
    : "editor-formula-kind-wrap";

  const kindBlock = (
    <div className={kindWrapClass} ref={kindAnchorRef}>
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
            setKindQuery(
              kind === "pending"
                ? ""
                : (ACTION_KIND_OPTIONS.find((opt) => opt.id === kind)?.label ?? kind),
            );
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
      {kindOpen && kindHits.length > 0 ? (
        <FormulaSuggestPortal anchorRef={kindAnchorRef}>
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
        </FormulaSuggestPortal>
      ) : null}
      {removeInKind ? removeButton : null}
    </div>
  );

  const argSlotClass = removeInArg
    ? "editor-formula-arg-slot editor-formula-arg-slot--clearable"
    : "editor-formula-arg-slot";

  const argBlock = isPending ? null : (
    <div className={argSlotClass}>
      {renderArg()}
      {removeInArg ? removeButton : null}
    </div>
  );

  const variableBridge = variableLabel ? (
    <div className="editor-formula-variable-bridge" aria-label={`${variableLabel} link`}>
      <svg
        className="editor-formula-variable-bracket-svg"
        viewBox="0 0 100 10"
        preserveAspectRatio="none"
        aria-hidden
      >
        <path
          d="M 0 0 L 0 8 L 100 8 L 100 0"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.25"
          vectorEffect="non-scaling-stroke"
        />
      </svg>
      <span className="editor-formula-variable-label">{variableLabel}</span>
    </div>
  ) : null;

  return (
    <div className="editor-formula-segment">
      <div className="editor-formula-segment-main">
        {variableLabel ? (
          <div className="editor-formula-variable-anchor">
            <div className="editor-formula-variable-inputs-row">
              {kindBlock}
              {argBlock}
            </div>
            {variableBridge}
          </div>
        ) : (
          <>
            {kindBlock}
            {argBlock}
          </>
        )}
      </div>
      {variableOpen && variableHits.length > 0 ? (
        <FormulaSuggestPortal anchorRef={variableAnchorRef}>
          {variableHits.map((label) => (
            <li key={label} role="none">
              <button
                type="button"
                role="option"
                className="editor-formula-suggest-btn"
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => applyVariableOption(label)}
              >
                <span className="editor-formula-suggest-title">{label}</span>
              </button>
            </li>
          ))}
        </FormulaSuggestPortal>
      ) : null}
    </div>
  );
}
