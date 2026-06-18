import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { createPortal } from "react-dom";
import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type FocusEvent,
  type ReactNode,
  type RefObject,
} from "react";
import type {
  CommandNodePayload,
  ComposerStatus,
  FormActionPayload,
  GenerateAutomationResult,
  NewToolDefinitionPayload,
  TestCommandResult,
  ActionPayload,
} from "../../types";
import { editorPendingAction, isEditorPendingAction } from "../../types";
import { formatUserError } from "../../utils/userErrors";
import { useEditorStore } from "../../store/editorStore";
import { useSettingsStore } from "../../store/settingsStore";
import { ACTION_KIND_OPTIONS, getActionKind } from "./actionCatalog";
import {
  appExeDisplayLabel,
  applyRemoveActionAt,
  computeInsetRemoveAllowed,
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
import { PlacementZoneSelect } from "./PlacementZoneSelect";
import { EditorSelect } from "../ui/EditorSelect";
import { CommandDeleteConfirm } from "./CommandDeleteConfirm";
import { ifElseConditionNeedsPattern, type IfConditionKind } from "./ifElse.logic";
import { EditorCheckIcon } from "./EditorCheckIcon";
import { EditorCloseXIcon } from "./EditorCloseXIcon";
import { EditorPlusIcon } from "./EditorPlusIcon";
import {
  canComposerGenerate,
  composerTriggerPayload,
  modelFromGeneratedResult,
  parseComposerInvokeError,
  shouldOfferRegenerateWithFix,
  shouldShowFormulaAfterGenerate,
  shouldShowToolPreview,
  toCreateToolPayload,
  toolDraftFromResult,
} from "./composer.logic";
import { EditorSpinner } from "./EditorSpinner";

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
    const rootPx = parseFloat(getComputedStyle(document.documentElement).fontSize) || 16;
    const minWidth = rootPx * 11; /* sync with --editor-dropdown-min-width */
    const width = Math.min(Math.max(r.width, minWidth), window.innerWidth - margin * 2);
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

const PREFIX_MODE_TOGGLE_TITLE =
  "Trailing words after the trigger become input for actions";

function PrefixModeToggle({
  checked,
  onChange,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label
      className={`editor-prefix-mode-toggle${checked ? " is-on" : ""}`}
      title={PREFIX_MODE_TOGGLE_TITLE}
    >
      <input
        type="checkbox"
        className="editor-prefix-mode-input"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        aria-label={PREFIX_MODE_TOGGLE_TITLE}
      />
      <span className="editor-prefix-mode-chip" aria-hidden>
        + input
      </span>
    </label>
  );
}

type FormulaTriggerColumnProps = {
  phrase: string;
  onPhraseChange: (value: string) => void;
  phrasePlaceholder?: string;
  prefixMode: boolean;
  onPrefixModeChange: (checked: boolean) => void;
  testInput?: string;
  onTestInputChange?: (value: string) => void;
};

function FormulaTriggerColumn({
  phrase,
  onPhraseChange,
  phrasePlaceholder = "Trigger phrase",
  prefixMode,
  onPrefixModeChange,
  testInput,
  onTestInputChange,
}: FormulaTriggerColumnProps) {
  return (
    <div className="editor-formula-trigger-wrap">
      <div className="editor-formula-trigger-head">
        <input
          type="text"
          className="editor-formula-input editor-formula-input--phrase"
          value={phrase}
          onChange={(e) => onPhraseChange(e.target.value)}
          placeholder={phrasePlaceholder}
          aria-label="Trigger phrase"
        />
        <PrefixModeToggle checked={prefixMode} onChange={onPrefixModeChange} />
      </div>
      {prefixMode && onTestInputChange ? (
        <input
          type="text"
          className="editor-formula-input editor-formula-input--test"
          value={testInput ?? ""}
          onChange={(e) => onTestInputChange(e.target.value)}
          placeholder="Test words…"
          aria-label="Prefix remainder test input"
        />
      ) : null}
    </div>
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
  const [deletePending, setDeletePending] = useState(false);
  const dirtyRef = useRef(false);
  const toastTimer = useRef<number | null>(null);
  const deleteCancelRef = useRef<HTMLButtonElement | null>(null);

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
      actions: applyRemoveActionAt(prev.actions, index),
    }));
  };
  const followUpVariableMeta = useMemo(() => deriveFollowUpVariableMap(model.actions), [model.actions]);
  const [testInput, setTestInput] = useState("");
  const [testRunning, setTestRunning] = useState(false);
  const [testResult, setTestResult] = useState<TestCommandResult | null>(null);

  const runTestCommand = useCallback(async () => {
    if (!model.id || testRunning) return;
    const errors = validateFormModel(model);
    if (hasBlockingErrors(errors)) {
      showToast("Fix validation errors before running.");
      return;
    }
    setTestRunning(true);
    setTestResult(null);
    try {
      const result = await invoke<TestCommandResult>("test_command", {
        commandId: model.id,
        testInput: model.prefixMode ? testInput : null,
      });
      setTestResult(result);
    } catch (err: unknown) {
      showToast(formatUserError(err, "Test run failed."));
    } finally {
      setTestRunning(false);
    }
  }, [model, showToast, testInput, testRunning]);

  const errors = validateFormModel(model);

  useEffect(() => {
    if (!deletePending) return;
    deleteCancelRef.current?.focus();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        setDeletePending(false);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [deletePending]);

  return (
    <li
      className={
        deletePending
          ? "editor-command-item editor-command-item--delete-pending"
          : "editor-command-item"
      }
    >
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
          <FormulaTriggerColumn
            phrase={primaryPhrase}
            onPhraseChange={setPrimaryPhrase}
            prefixMode={model.prefixMode}
            onPrefixModeChange={(checked) =>
              updateModel((prev) => ({ ...prev, prefixMode: checked }))
            }
            testInput={testInput}
            onTestInputChange={setTestInput}
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
                    availableVariableLabels={deriveFormulaVariableLabels(
                      model.actions,
                      model.prefixMode,
                      index,
                    )}
                    variableLabel={
                      followUpVariableMeta.byActionIndex.get(index)
                        ? `Variable ${followUpVariableMeta.byActionIndex.get(index)}`
                        : undefined
                    }
                    onChange={(next) => setActionAt(index, next)}
                    onRemove={() => removeActionAt(index)}
                    canRemove={computeInsetRemoveAllowed(model.actions)}
                    prefixMode={model.prefixMode}
                    formulaActions={model.actions}
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
              <span className="editor-formula-plus-icon" aria-hidden>
                <EditorPlusIcon className="editor-formula-plus-icon-svg" />
              </span>
            </button>
          </div>

          <div className="editor-command-trail">
            <button
              type="button"
              className="editor-formula-run"
              onClick={() => void runTestCommand()}
              disabled={!model.id || testRunning}
            >
              {testRunning ? "Running…" : "Run"}
            </button>
            <button
              type="button"
              className={`editor-switch${model.enabled ? " is-on" : ""}`}
              role="switch"
              aria-checked={model.enabled}
              onClick={onToggleEnabled}
            >
              <span className="editor-switch-knob" />
            </button>
            <CommandDeleteConfirm
              deletePending={deletePending}
              phraseLabel={primaryPhrase}
              onRequestDelete={() => setDeletePending(true)}
              onCancel={() => setDeletePending(false)}
              onConfirm={() => {
                setDeletePending(false);
                onDelete();
              }}
              cancelRef={deleteCancelRef}
            />
          </div>
        </div>

        {(errors.actions || errors.triggerPhrases || Object.keys(errors.actionErrors).length > 0) && (
          <p className="editor-field-error editor-command-row-errors">
            {[errors.triggerPhrases, errors.actions, ...Object.values(errors.actionErrors)]
              .filter(Boolean)
              .join(" ")}
          </p>
        )}
        {testResult && testResult.steps.length > 0 && (
          <div className="editor-formula-run-strip" role="status" aria-live="polite">
            {testResult.steps.map((step) => (
              <span
                key={`run-${step.index}-${step.action_kind}`}
                className={`editor-formula-run-chip${step.error ? " is-error" : ""}`}
                title={step.error ?? step.status}
              >
                {step.action_kind}: {step.error ?? (step.output_preview || step.status)}
              </span>
            ))}
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
  const [composerTrigger, setComposerTrigger] = useState("");
  const [description, setDescription] = useState("");
  const [generating, setGenerating] = useState(false);
  const [composerStatus, setComposerStatus] = useState<ComposerStatus | null>(null);
  const [generateResult, setGenerateResult] = useState<GenerateAutomationResult | null>(null);
  const [formulaVisible, setFormulaVisible] = useState(false);
  const [toolPreviewVisible, setToolPreviewVisible] = useState(false);
  const [saving, setSaving] = useState(false);
  const [toastText, setToastText] = useState<string | null>(null);
  const [showRegenerateFix, setShowRegenerateFix] = useState(false);

  const refreshComposerStatus = useCallback(() => {
    void invoke<ComposerStatus>("composer_status")
      .then(setComposerStatus)
      .catch(() => {});
  }, []);

  useEffect(() => {
    let cancelled = false;
    void invoke<ComposerStatus>("composer_status")
      .then((status) => {
        if (!cancelled) setComposerStatus(status);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen("composer-warmup", () => {
      refreshComposerStatus();
    }).then((off) => {
      unlisten = off;
    });
    return () => {
      unlisten?.();
    };
  }, [refreshComposerStatus]);

  const onGenerate = async () => {
    const text = description.trim();
    if (!canComposerGenerate(composerTrigger, description) || generating) return;
    setGenerating(true);
    setToastText(null);
    setShowRegenerateFix(false);
    refreshComposerStatus();
    try {
      const result = await invoke<GenerateAutomationResult>("generate_automation", {
        description: text,
        trigger: composerTriggerPayload(composerTrigger),
      });
      setGenerateResult(result);
      const nextModel = modelFromGeneratedResult(result);
      if (nextModel) {
        setModel(nextModel);
      }
      setFormulaVisible(shouldShowFormulaAfterGenerate(result));
      setToolPreviewVisible(shouldShowToolPreview(result));
      setShowRegenerateFix(false);
    } catch (err: unknown) {
      const parsed = parseComposerInvokeError(err);
      setShowRegenerateFix(shouldOfferRegenerateWithFix(err));
      setToastText(formatUserError(parsed.message, "Could not compose command."));
    } finally {
      setGenerating(false);
      refreshComposerStatus();
    }
  };

  const onCancelGenerate = () => {
    void invoke("cancel_generate_automation");
    setGenerating(false);
  };

  const composerBusy = generating || Boolean(composerStatus?.loading);
  const statusMessage = generating
    ? "Composing…"
    : composerStatus?.loading
      ? "Model loading…"
      : null;

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
      const toolDraft =
        generateResult?.kind === "both" ? toolDraftFromResult(generateResult) : null;
      if (toolDraft) {
        await invoke<NewToolDefinitionPayload>("create_tool", {
          tool: toCreateToolPayload(toolDraft),
        });
      }
      onCreated();
    } catch (err: unknown) {
      setToastText(formatUserError(err, "Could not create command."));
    } finally {
      setSaving(false);
    }
  };

  const onCreateTool = async () => {
    if (!generateResult) return;
    const toolDraft = toolDraftFromResult(generateResult);
    if (!toolDraft) {
      setToastText("No tool draft to save.");
      return;
    }
    setSaving(true);
    try {
      await invoke<NewToolDefinitionPayload>("create_tool", {
        tool: toCreateToolPayload(toolDraft),
      });
      onCreated();
    } catch (err: unknown) {
      setToastText(formatUserError(err, "Could not create tool."));
    } finally {
      setSaving(false);
    }
  };

  const toolDraft = generateResult ? toolDraftFromResult(generateResult) : null;

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
      actions: applyRemoveActionAt(prev.actions, index),
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
        <div
          className="editor-composer-panel"
          aria-busy={composerBusy}
        >
          <label className="editor-composer-label" htmlFor="command-composer-trigger">
            Trigger
          </label>
          <input
            id="command-composer-trigger"
            type="text"
            className="editor-composer-input"
            value={composerTrigger}
            onChange={(e) => setComposerTrigger(e.target.value)}
            placeholder='e.g. open notepad'
            disabled={generating}
            autoComplete="off"
            spellCheck={false}
          />
          <label className="editor-composer-label" htmlFor="command-composer-description">
            What should happen
          </label>
          <textarea
            id="command-composer-description"
            className="editor-composer-textarea"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder="e.g. launch Notepad snapped left"
            rows={3}
            disabled={generating}
          />
          <div className="editor-composer-actions">
            {generating ? (
              <button
                type="button"
                className="editor-composer-btn editor-composer-btn--ghost"
                onClick={onCancelGenerate}
              >
                Cancel
              </button>
            ) : null}
            {showRegenerateFix ? (
              <button
                type="button"
                className="editor-composer-btn editor-composer-btn--ghost"
                onClick={() => void onGenerate()}
                disabled={generating || !canComposerGenerate(composerTrigger, description)}
              >
                Regenerate with fix
              </button>
            ) : null}
            <button
              type="button"
              className="editor-composer-btn editor-composer-btn--secondary"
              onClick={() => void onGenerate()}
              disabled={generating || !canComposerGenerate(composerTrigger, description)}
              aria-busy={generating}
            >
              {generating ? (
                <>
                  <EditorSpinner size={16} />
                  <span>Composing…</span>
                </>
              ) : (
                "Generate"
              )}
            </button>
          </div>
          {(statusMessage || generateResult) && (
            <div className="editor-composer-status" aria-live="polite">
              {statusMessage ? (
                <>
                  <EditorSpinner size={14} />
                  <span>{statusMessage}</span>
                </>
              ) : null}
              {generateResult && !statusMessage ? (
                <>
                  <span className="editor-composer-summary">{generateResult.summary}</span>
                  {generateResult.warnings.map((warning) => (
                    <span key={warning} className="editor-composer-warning">
                      {warning}
                    </span>
                  ))}
                </>
              ) : null}
            </div>
          )}
          {composerStatus && !composerStatus.modelPresent && composerStatus.featureCompiled ? (
            <p className="editor-composer-hint">
              Composer model missing — run <code>npm run fetch-models</code> from the jarvis folder.
            </p>
          ) : null}
        </div>
        {toolPreviewVisible && !formulaVisible && toolDraft ? (
          <div className="editor-composer-tool-only editor-command-formula--revealed">
            <ComposerToolPreview tool={toolDraft} variant="card" />
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
                className="editor-composer-btn editor-composer-btn--secondary"
                onClick={() => void onCreateTool()}
                disabled={saving}
                aria-busy={saving}
              >
                {saving ? (
                  <>
                    <EditorSpinner size={14} />
                    <span>Creating…</span>
                  </>
                ) : (
                  "Create tool"
                )}
              </button>
            </div>
          </div>
        ) : null}
        {formulaVisible ? (
        <div className="editor-command-formula editor-command-formula--revealed">
          {toolPreviewVisible && toolDraft ? (
            <ComposerToolPreview tool={toolDraft} variant="callout" />
          ) : null}
          <FormulaTriggerColumn
            phrase={primaryPhrase}
            onPhraseChange={setPrimaryPhrase}
            phrasePlaceholder="Phrase"
            prefixMode={model.prefixMode}
            onPrefixModeChange={(checked) =>
              updateModel((prev) => ({ ...prev, prefixMode: checked }))
            }
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
                  availableVariableLabels={deriveFormulaVariableLabels(
                    model.actions,
                    model.prefixMode,
                    index,
                  )}
                  variableLabel={
                    followUpVariableMeta.byActionIndex.get(index)
                      ? `Variable ${followUpVariableMeta.byActionIndex.get(index)}`
                      : undefined
                  }
                  onChange={(next) => setActionAt(index, next)}
                  onRemove={() => removeActionAt(index)}
                  canRemove={computeInsetRemoveAllowed(model.actions)}
                  prefixMode={model.prefixMode}
                  formulaActions={model.actions}
                />
              </div>
            ))}
            <button type="button" className="editor-formula-plus" onClick={addActionSegment} aria-label="Add action">
              <span className="editor-formula-plus-icon" aria-hidden>
                <EditorPlusIcon className="editor-formula-plus-icon-svg" />
              </span>
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
              className="editor-command-draft-icon-btn"
              onClick={() => void onSave()}
              disabled={saving}
              aria-label={saving ? "Saving…" : "Save"}
            >
              <span className="editor-command-draft-icon" aria-hidden>
                {saving ? <DraftBusyIcon /> : <EditorCheckIcon className="editor-command-draft-icon-svg" />}
              </span>
            </button>
          </div>
        </div>
        ) : null}
      </div>
    </li>
  );
}

function ComposerToolPreview({
  tool,
  variant,
}: {
  tool: NewToolDefinitionPayload;
  variant: "card" | "callout";
}) {
  const paramSummary =
    tool.parameters.length > 0 ? tool.parameters.map((param) => param.name).join(", ") : "none";
  const className =
    variant === "card" ? "editor-composer-tool-preview" : "editor-composer-tool-callout";
  return (
    <div className={className}>
      <p className="editor-composer-tool-title">{tool.display_name}</p>
      <p className="editor-composer-tool-meta">
        LLM tool <code>{tool.name}</code>
        {" · "}
        params: {paramSummary}
      </p>
      {tool.description ? (
        <p className="editor-composer-tool-desc">{tool.description}</p>
      ) : null}
      {variant === "callout" ? (
        <p className="editor-composer-tool-hint">Save also creates this tool.</p>
      ) : null}
    </div>
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
  /** Hide kinds in nested If/Else branches (avoids deep nesting in v1). */
  excludeActionKinds?: ConcreteActionKind[];
  prefixMode?: boolean;
  formulaActions?: FormActionPayload[];
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

export function deriveFormulaVariableLabels(
  actions: FormActionPayload[],
  prefixMode: boolean,
  forActionIndex: number,
): string[] {
  const followUp = deriveFollowUpVariableMap(actions);
  const labels = [...followUp.labels, "{{last_result}}"];
  if (prefixMode) {
    labels.push("{{remainder}}", "{{shortcut_input}}");
  }
  for (let i = 0; i < forActionIndex; i += 1) {
    labels.push(`{{step_${i + 1}}}`);
  }
  return labels;
}

export function extractVariableTokenContext(inputValue: string, caret: number): VariableTokenContext | null {
  const before = inputValue.slice(0, caret);
  const varMatch = /(^|\s)(Variable(?:\s+\d*)?)$/i.exec(before);
  if (varMatch) {
    const token = varMatch[2];
    return {
      start: before.length - token.length,
      end: caret,
      query: token.replace(/^Variable/i, "").trim(),
    };
  }
  const tplMatch = /\{\{([a-z_]*)$/i.exec(before);
  if (tplMatch) {
    return {
      start: before.length - tplMatch[0].length,
      end: caret,
      query: tplMatch[1] ?? "",
    };
  }
  return null;
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
  excludeActionKinds = [],
  prefixMode = false,
  formulaActions,
}: SegmentProps) {
  const commandOptions = useEditorStore((s) => s.nodes);
  const kindAnchorRef = useRef<HTMLDivElement>(null);
  const appAnchorRef = useRef<HTMLDivElement>(null);
  const appInputRef = useRef<HTMLInputElement>(null);
  const variableAnchorRef = useRef<HTMLDivElement>(null);
  const variableTargetRef = useRef<{
    value: string;
    start: number;
    end: number;
    onCommit: (next: string) => void;
    input: HTMLInputElement;
  } | null>(null);
  const kind = getActionKind(action);
  const fieldIds = useId();
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
  /** Drive ✕ + padding when CSS :hover is flaky (nested flex / WebView); keep true while focus inside slot. */
  const [kindInsetHover, setKindInsetHover] = useState(false);
  const [kindInsetFocusInside, setKindInsetFocusInside] = useState(false);
  const [argInsetHover, setArgInsetHover] = useState(false);
  const [argInsetFocusInside, setArgInsetFocusInside] = useState(false);
  const [appEditing, setAppEditing] = useState(
    () => !("open_app" in action) || action.open_app.path.trim().length === 0,
  );
  const [selectedAppIcon, setSelectedAppIcon] = useState<string | null>(null);
  const appTimer = useRef<number | null>(null);
  /** Latest `action` for async blur handlers (avoid stale closures). */
  const latestActionRef = useRef(action);
  latestActionRef.current = action;
  const collapseAppEditor = useCallback(() => {
    setAppOpen(false);
    setAppHasSearched(false);
    const a = latestActionRef.current;
    if ("open_app" in a && a.open_app.path.trim().length > 0) {
      setAppQuery(a.open_app.name);
      setAppEditing(false);
    }
  }, []);
  const argSlotDomRef = useRef<HTMLDivElement | null>(null);

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
    const pool = ACTION_KIND_OPTIONS.filter((opt) => !excludeActionKinds.includes(opt.id));
    if (!q) return pool;
    return pool.filter(
      (opt) =>
        opt.label.toLowerCase().includes(q) ||
        opt.haystack.includes(q) ||
        opt.id.split("_").join(" ").includes(q),
    );
  }, [kindQuery, excludeActionKinds]);

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

  useEffect(() => {
    if (!appEditing) return;
    const a = latestActionRef.current;
    if (!("open_app" in a) || a.open_app.path.trim().length === 0) return;
    const el = appInputRef.current;
    if (!el) return;
    const id = requestAnimationFrame(() => {
      el.focus({ preventScroll: true });
    });
    return () => cancelAnimationFrame(id);
  }, [appEditing]);

  useEffect(() => {
    if (!appEditing) return;
    const a = latestActionRef.current;
    if (!("open_app" in a) || a.open_app.path.trim().length === 0) return;

    const onPointerDown = (e: PointerEvent) => {
      const target = e.target;
      if (!(target instanceof Node)) return;
      if (appAnchorRef.current?.contains(target)) return;
      if (target instanceof Element && target.closest(".editor-formula-suggest")) return;
      collapseAppEditor();
    };

    document.addEventListener("pointerdown", onPointerDown, true);
    return () => document.removeEventListener("pointerdown", onPointerDown, true);
  }, [appEditing, collapseAppEditor, action]);

  const variableHits = useMemo(() => {
    if (!availableVariableLabels.length) return [];
    const q = variableQuery.trim().toLowerCase();
    return availableVariableLabels.filter((label) => {
      if (label.startsWith("{{")) {
        if (!q) return true;
        const inner = label.slice(2, label.endsWith("}}") ? -2 : undefined).toLowerCase();
        return inner.startsWith(q) || label.toLowerCase().startsWith(`{{${q}`);
      }
      if (!q) return true;
      return label.toLowerCase().startsWith(`variable ${q}`);
    });
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
      const showAppLeadingIcon = action.open_app.path.trim().length > 0;
      const appConfirmed = appDisplayMode === "confirmed";
      const appInputClass = [
        formulaArgInputClass(),
        appConfirmed ? "editor-formula-input--app-confirmed" : null,
      ]
        .filter(Boolean)
        .join(" ");
      return (
        <div className="editor-formula-arg-wrap editor-formula-arg-wrap--with-placement">
        <div
          className={
            showAppLeadingIcon
              ? "editor-formula-arg-wrap editor-formula-arg-wrap--leading-app-icon"
              : "editor-formula-arg-wrap"
          }
          ref={(el) => {
            appAnchorRef.current = el;
            variableAnchorRef.current = el;
          }}
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
            ref={appInputRef}
            type="text"
            className={appInputClass}
            value={appQuery}
            readOnly={appConfirmed}
            title={appConfirmed ? action.open_app.name || "App" : undefined}
            {...(!appConfirmed
              ? bindVariableSuggestInput(appQuery, (value) => {
                  setAppQuery(value);
                  setAppEditing(true);
                  onChange({
                    open_app: { name: value, path: "", placement: action.open_app.placement },
                  });
                })
              : {})}
            onFocus={() => {
              setAppEditing(true);
              setAppOpen(true);
            }}
            onBlur={() => window.setTimeout(() => collapseAppEditor(), 120)}
            placeholder="Search app…"
            aria-label={
              appConfirmed
                ? `Selected app ${action.open_app.name}. Click to change app.`
                : `App name for step ${index + 1}`
            }
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
                        onChange({
                          open_app: {
                            name: h.display_name,
                            path: h.exe_path,
                            placement: action.open_app.placement,
                          },
                        });
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
        {appConfirmed ? (
          <PlacementZoneSelect
            value={action.open_app.placement}
            ariaLabel={`Placement for step ${index + 1}`}
            onChange={(placement) =>
              onChange({
                open_app: { ...action.open_app, placement },
              })
            }
          />
        ) : null}
        </div>
      );
    }
    if ("open_target" in action) {
      return (
        <div className="editor-formula-arg-wrap editor-formula-arg-wrap--with-placement" ref={variableAnchorRef}>
          <input
            type="text"
            className={formulaArgInputClass()}
            value={action.open_target.target}
            {...bindVariableSuggestInput(action.open_target.target, (value) =>
              onChange({ open_target: { ...action.open_target, target: value } }),
            )}
            placeholder="App or site name"
            aria-label={`Open target for step ${index + 1}`}
          />
          <PlacementZoneSelect
            value={action.open_target.placement}
            ariaLabel={`Placement for step ${index + 1}`}
            onChange={(placement) =>
              onChange({ open_target: { ...action.open_target, placement } })
            }
          />
        </div>
      );
    }
    if ("place_window" in action) {
      return (
        <PlacementZoneSelect
          allowEmpty={false}
          value={action.place_window.zone}
          ariaLabel={`Snap zone for step ${index + 1}`}
          onChange={(zone) =>
            onChange({
              place_window: { ...action.place_window, zone: zone ?? "right_half" },
            })
          }
        />
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
    if ("run_command" in action) {
      return (
        <div className="editor-formula-arg-wrap" ref={variableAnchorRef}>
          <EditorSelect
            id={`${fieldIds}-run-command`}
            value={action.run_command.command_id ? String(action.run_command.command_id) : ""}
            onChange={(next) =>
              onChange({
                run_command: {
                  ...action.run_command,
                  command_id: Number(next) || 0,
                },
              })
            }
            options={commandOptions.map((node) => ({
              value: String(node.id),
              label: node.trigger_phrases[0] ?? node.name,
            }))}
            placeholder="Select command…"
            ariaLabel={`Command to run for step ${index + 1}`}
            className="editor-select-wrap--formula"
          />
          <input
            type="text"
            className={formulaArgInputClass()}
            value={action.run_command.input ?? ""}
            {...bindVariableSuggestInput(action.run_command.input ?? "", (value) =>
              onChange({
                run_command: {
                  ...action.run_command,
                  input: value,
                },
              }),
            )}
            placeholder="Input (optional)"
            aria-label={`Run command input for step ${index + 1}`}
          />
        </div>
      );
    }
    if ("read_file" in action) {
      return (
        <div className="editor-formula-arg-wrap" ref={variableAnchorRef}>
          <input
            type="text"
            className={formulaArgInputClass()}
            value={action.read_file.path}
            {...bindVariableSuggestInput(action.read_file.path, (value) =>
              onChange({ read_file: { path: value } }),
            )}
            placeholder="File path"
            aria-label={`File path for step ${index + 1}`}
          />
        </div>
      );
    }
    if ("http_get" in action) {
      return (
        <div className="editor-formula-arg-wrap" ref={variableAnchorRef}>
          <input
            type="url"
            className={formulaArgInputClass()}
            value={action.http_get.url}
            {...bindVariableSuggestInput(action.http_get.url, (value) =>
              onChange({ http_get: { url: value } }),
            )}
            placeholder="https://…"
            aria-label={`HTTP GET URL for step ${index + 1}`}
          />
        </div>
      );
    }
    if ("get_clipboard" in action) {
      return (
        <span className="editor-formula-muted" aria-label={`Clipboard read for step ${index + 1}`}>
          System clipboard
        </span>
      );
    }
    if ("show_notification" in action) {
      return (
        <div
          className="editor-formula-arg-wrap editor-formula-arg-wrap--stack"
          ref={variableAnchorRef}
        >
          <input
            type="text"
            className={formulaArgInputClass()}
            value={action.show_notification.title}
            {...bindVariableSuggestInput(action.show_notification.title, (value) =>
              onChange({
                show_notification: { ...action.show_notification, title: value },
              }),
            )}
            placeholder="Title"
            aria-label={`Notification title for step ${index + 1}`}
          />
          <input
            type="text"
            className={formulaArgInputClass()}
            value={action.show_notification.body}
            {...bindVariableSuggestInput(action.show_notification.body, (value) =>
              onChange({
                show_notification: { ...action.show_notification, body: value },
              }),
            )}
            placeholder="Body"
            aria-label={`Notification body for step ${index + 1}`}
          />
        </div>
      );
    }
    if ("text_trim" in action) {
      return (
        <div className="editor-formula-arg-wrap" ref={variableAnchorRef}>
          <input
            type="text"
            className={formulaArgInputClass()}
            value={action.text_trim.text}
            {...bindVariableSuggestInput(action.text_trim.text, (value) =>
              onChange({ text_trim: { text: value } }),
            )}
            placeholder="Text (optional — uses prior output)"
            aria-label={`Trim text input for step ${index + 1}`}
          />
        </div>
      );
    }
    if ("text_match" in action) {
      return (
        <div
          className="editor-formula-arg-wrap editor-formula-arg-wrap--stack"
          ref={variableAnchorRef}
        >
          <input
            type="text"
            className={formulaArgInputClass()}
            value={action.text_match.pattern}
            {...bindVariableSuggestInput(action.text_match.pattern, (value) =>
              onChange({
                text_match: { ...action.text_match, pattern: value },
              }),
            )}
            placeholder="Regex pattern"
            aria-label={`Match pattern for step ${index + 1}`}
          />
          <input
            type="text"
            className={formulaArgInputClass()}
            value={action.text_match.text}
            {...bindVariableSuggestInput(action.text_match.text, (value) =>
              onChange({
                text_match: { ...action.text_match, text: value },
              }),
            )}
            placeholder="Text (optional — uses prior output)"
            aria-label={`Match text input for step ${index + 1}`}
          />
        </div>
      );
    }
    if ("text_split" in action) {
      return (
        <div
          className="editor-formula-arg-wrap editor-formula-arg-wrap--stack"
          ref={variableAnchorRef}
        >
          <input
            type="text"
            className={formulaArgInputClass()}
            value={action.text_split.delimiter}
            {...bindVariableSuggestInput(action.text_split.delimiter, (value) =>
              onChange({
                text_split: { ...action.text_split, delimiter: value },
              }),
            )}
            placeholder="Delimiter"
            aria-label={`Split delimiter for step ${index + 1}`}
          />
          <input
            type="text"
            className={formulaArgInputClass()}
            value={action.text_split.text}
            {...bindVariableSuggestInput(action.text_split.text, (value) =>
              onChange({
                text_split: { ...action.text_split, text: value },
              }),
            )}
            placeholder="Text (optional — uses prior output)"
            aria-label={`Split text input for step ${index + 1}`}
          />
        </div>
      );
    }
    if ("text_combine" in action) {
      return (
        <div className="editor-formula-arg-wrap" ref={variableAnchorRef}>
          <input
            type="text"
            className={formulaArgInputClass()}
            value={action.text_combine.separator}
            {...bindVariableSuggestInput(action.text_combine.separator, (value) =>
              onChange({ text_combine: { separator: value } }),
            )}
            placeholder="Separator"
            aria-label={`Combine separator for step ${index + 1}`}
          />
        </div>
      );
    }
    if ("set_clipboard" in action) {
      return (
        <div className="editor-formula-arg-wrap" ref={variableAnchorRef}>
          <input
            type="text"
            className={formulaArgInputClass()}
            value={action.set_clipboard.text}
            {...bindVariableSuggestInput(action.set_clipboard.text, (value) =>
              onChange({ set_clipboard: { text: value } }),
            )}
            placeholder="Text (optional — uses prior output)"
            aria-label={`Set clipboard text for step ${index + 1}`}
          />
        </div>
      );
    }
    if ("list_folder" in action) {
      return (
        <div className="editor-formula-arg-wrap" ref={variableAnchorRef}>
          <input
            type="text"
            className={formulaArgInputClass()}
            value={action.list_folder.path}
            {...bindVariableSuggestInput(action.list_folder.path, (value) =>
              onChange({ list_folder: { path: value } }),
            )}
            placeholder="Folder path"
            aria-label={`Folder path for step ${index + 1}`}
          />
        </div>
      );
    }
    if ("write_file" in action) {
      return (
        <div
          className="editor-formula-arg-wrap editor-formula-arg-wrap--stack"
          ref={variableAnchorRef}
        >
          <input
            type="text"
            className={formulaArgInputClass()}
            value={action.write_file.path}
            {...bindVariableSuggestInput(action.write_file.path, (value) =>
              onChange({
                write_file: { ...action.write_file, path: value },
              }),
            )}
            placeholder="File path"
            aria-label={`Write file path for step ${index + 1}`}
          />
          <input
            type="text"
            className={formulaArgInputClass()}
            value={action.write_file.content}
            {...bindVariableSuggestInput(action.write_file.content, (value) =>
              onChange({
                write_file: { ...action.write_file, content: value },
              }),
            )}
            placeholder="Content (optional — uses prior output)"
            aria-label={`Write file content for step ${index + 1}`}
          />
        </div>
      );
    }
    if ("get_file_metadata" in action) {
      return (
        <div className="editor-formula-arg-wrap" ref={variableAnchorRef}>
          <input
            type="text"
            className={formulaArgInputClass()}
            value={action.get_file_metadata.path}
            {...bindVariableSuggestInput(action.get_file_metadata.path, (value) =>
              onChange({ get_file_metadata: { path: value } }),
            )}
            placeholder="File or folder path"
            aria-label={`Metadata path for step ${index + 1}`}
          />
        </div>
      );
    }
    if ("screenshot" in action) {
      return (
        <div className="editor-formula-arg-wrap" ref={variableAnchorRef}>
          <input
            type="text"
            className={formulaArgInputClass()}
            value={action.screenshot.path ?? ""}
            {...bindVariableSuggestInput(action.screenshot.path ?? "", (value) =>
              onChange({ screenshot: { path: value } }),
            )}
            placeholder="Output path (optional — temp file)"
            aria-label={`Screenshot output path for step ${index + 1}`}
          />
        </div>
      );
    }
    if ("device_info" in action) {
      return (
        <span className="editor-formula-muted" aria-label={`Device info for step ${index + 1}`}>
          CPU &amp; memory
        </span>
      );
    }
    if ("start_dictation" in action) {
      return (
        <span className="editor-formula-muted" aria-label={`Start dictation for step ${index + 1}`}>
          Voice typing on
        </span>
      );
    }
    if ("stop_dictation" in action) {
      return (
        <span className="editor-formula-muted" aria-label={`Stop dictation for step ${index + 1}`}>
          Voice typing off
        </span>
      );
    }
    if ("if_else" in action) {
      const needsPattern = ifElseConditionNeedsPattern(action.if_else.condition);
      return (
        <div
          className="editor-formula-arg-wrap editor-formula-arg-wrap--stack"
          ref={variableAnchorRef}
        >
          <EditorSelect
            id={`${fieldIds}-if-condition`}
            value={action.if_else.condition}
            onChange={(next) =>
              onChange({
                if_else: {
                  ...action.if_else,
                  condition: next as IfConditionKind,
                },
              })
            }
            options={[
              { value: "text_contains", label: "Text contains" },
              { value: "regex_match", label: "Matches regex" },
              { value: "text_is_empty", label: "Is empty" },
            ]}
            ariaLabel={`If/Else condition for step ${index + 1}`}
            className="editor-select-wrap--formula"
          />
          <input
            type="text"
            className={formulaArgInputClass()}
            value={action.if_else.text}
            {...bindVariableSuggestInput(action.if_else.text, (value) =>
              onChange({
                if_else: { ...action.if_else, text: value },
              }),
            )}
            placeholder="Text (optional — uses prior output)"
            aria-label={`If/Else text for step ${index + 1}`}
          />
          {needsPattern ? (
            <input
              type="text"
              className={formulaArgInputClass()}
              value={action.if_else.pattern}
              {...bindVariableSuggestInput(action.if_else.pattern, (value) =>
                onChange({
                  if_else: { ...action.if_else, pattern: value },
                }),
              )}
              placeholder={action.if_else.condition === "regex_match" ? "Regex" : "Contains"}
              aria-label={`If/Else pattern for step ${index + 1}`}
            />
          ) : null}
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

  const isPending = "editor_pending" in action;
  /** Pending rows have no arg field — inset remove belongs on the kind ("Action") control. */
  const removeInKind = canRemove && isPending;
  const removeInArg = canRemove && !isPending;

  const kindInsetHot = kindInsetHover || kindInsetFocusInside;
  const argInsetHot = argInsetHover || argInsetFocusInside;

  const kindWrapRefCallback = useCallback(
    (el: HTMLDivElement | null) => {
      kindAnchorRef.current = el;
      if (el && removeInKind) {
        try {
          if (el.matches(":hover")) setKindInsetHover(true);
        } catch {
          /* :hover can throw in non-DOM test envs */
        }
      }
    },
    [removeInKind],
  );

  const argSlotRefCallback = useCallback(
    (el: HTMLDivElement | null) => {
      argSlotDomRef.current = el;
      if (el && removeInArg) {
        try {
          if (el.matches(":hover")) setArgInsetHover(true);
        } catch {
          /* noop */
        }
      }
    },
    [removeInArg],
  );

  const openAppPathForLayout =
    "open_app" in action ? action.open_app.path : "";

  useLayoutEffect(() => {
    try {
      const k = kindAnchorRef.current;
      if (k && removeInKind && k.matches(":hover")) setKindInsetHover(true);
      const slot = argSlotDomRef.current;
      if (slot && removeInArg && slot.matches(":hover")) setArgInsetHover(true);
    } catch {
      /* noop */
    }
  }, [
    removeInKind,
    removeInArg,
    kind,
    appDisplayMode,
    appEditing,
    openAppPathForLayout,
  ]);

  const removeButton = () =>
    canRemove ? (
      <button
        type="button"
        className="editor-formula-remove-inline"
        tabIndex={-1}
        onClick={onRemove}
        aria-label={`Remove step ${index + 1}`}
      >
        <EditorCloseXIcon className="editor-formula-remove-inline-x" />
      </button>
    ) : null;

  const kindWrapClass = [
    "editor-formula-kind-wrap",
    removeInKind ? "editor-formula-kind-wrap--clearable" : null,
    removeInKind && kindInsetHot ? "editor-formula-kind-wrap--hot" : null,
  ]
    .filter(Boolean)
    .join(" ");

  const kindChromeHandlers = removeInKind
    ? {
        onMouseEnter: () => setKindInsetHover(true),
        onMouseLeave: () => setKindInsetHover(false),
        onFocusCapture: () => setKindInsetFocusInside(true),
        onBlurCapture: (e: FocusEvent<HTMLDivElement>) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node | null)) {
            setKindInsetFocusInside(false);
          }
        },
      }
    : {};

  const kindBlock = (
    <div className={kindWrapClass} ref={kindWrapRefCallback} {...kindChromeHandlers}>
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
      {removeInKind ? removeButton() : null}
    </div>
  );

  const argSlotClass = [
    "editor-formula-arg-slot",
    removeInArg ? "editor-formula-arg-slot--clearable" : null,
    removeInArg && argInsetHot ? "editor-formula-arg-slot--hot" : null,
  ]
    .filter(Boolean)
    .join(" ");

  const argChromeHandlers = removeInArg
    ? {
        onMouseEnter: () => setArgInsetHover(true),
        onMouseLeave: () => setArgInsetHover(false),
        onFocusCapture: () => setArgInsetFocusInside(true),
        onBlurCapture: (e: FocusEvent<HTMLDivElement>) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node | null)) {
            setArgInsetFocusInside(false);
          }
        },
      }
    : {};

  const argBlock = isPending ? null : (
    <div className={argSlotClass} ref={argSlotRefCallback} {...argChromeHandlers}>
      {renderArg()}
      {removeInArg ? removeButton() : null}
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
      {"if_else" in action ? (
        <div className="editor-formula-ifelse-branches" role="group" aria-label="If/Else branches">
          <IfElseBranchPanel
            label="Then"
            actions={action.if_else.then_actions}
            onChange={(then_actions) =>
              onChange({ if_else: { ...action.if_else, then_actions } })
            }
            parentStepIndex={index}
            prefixMode={prefixMode}
            formulaActions={formulaActions ?? []}
          />
          <IfElseBranchPanel
            label="Else"
            actions={action.if_else.else_actions}
            onChange={(else_actions) =>
              onChange({ if_else: { ...action.if_else, else_actions } })
            }
            parentStepIndex={index}
            prefixMode={prefixMode}
            formulaActions={formulaActions ?? []}
          />
        </div>
      ) : null}
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

type IfElseBranchPanelProps = {
  label: string;
  actions: ActionPayload[];
  onChange: (next: ActionPayload[]) => void;
  parentStepIndex: number;
  prefixMode: boolean;
  formulaActions: FormActionPayload[];
};

function IfElseBranchPanel({
  label,
  actions,
  onChange,
  parentStepIndex,
  prefixMode,
  formulaActions,
}: IfElseBranchPanelProps) {
  const setBranchActionAt = (branchIndex: number, next: FormActionPayload) => {
    if (isEditorPendingAction(next)) return;
    const copy = [...actions];
    copy[branchIndex] = next;
    onChange(copy);
  };

  const removeBranchActionAt = (branchIndex: number) => {
    onChange(actions.filter((_, i) => i !== branchIndex));
  };

  return (
    <div className="editor-formula-branch">
      <div className="editor-formula-branch-bridge" aria-hidden>
        <svg className="editor-formula-variable-bracket-svg" viewBox="0 0 100 10" preserveAspectRatio="none">
          <path
            d="M 0 0 L 0 8 L 100 8 L 100 0"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.25"
            vectorEffect="non-scaling-stroke"
          />
        </svg>
        <span className="editor-formula-variable-label">{label}</span>
      </div>
      <div className="editor-formula-branch-chain" role="group" aria-label={`${label} branch actions`}>
        {actions.length === 0 ? (
          <span className="editor-formula-muted">No actions</span>
        ) : (
          actions.map((branchAction, branchIndex) => (
            <div key={`${label}-${branchIndex}`} className="editor-formula-branch-segment-wrap">
              {branchIndex > 0 ? (
                <span className="editor-formula-arrow" aria-hidden>
                  +
                </span>
              ) : null}
              <ActionSegmentEditor
                action={branchAction}
                index={branchIndex}
                availableVariableLabels={deriveFormulaVariableLabels(
                  formulaActions,
                  prefixMode,
                  parentStepIndex,
                )}
                onChange={(next) => setBranchActionAt(branchIndex, next)}
                onRemove={() => removeBranchActionAt(branchIndex)}
                canRemove
                excludeActionKinds={["if_else"]}
                prefixMode={prefixMode}
                formulaActions={formulaActions}
              />
            </div>
          ))
        )}
        <button
          type="button"
          className="editor-formula-plus editor-formula-plus--labeled"
          onClick={() => onChange([...actions, { speak: { text: "" } }])}
          aria-label={`Add ${label} action`}
        >
          <span className="editor-formula-plus-icon" aria-hidden>
            <EditorPlusIcon className="editor-formula-plus-icon-svg" />
          </span>
        </button>
      </div>
    </div>
  );
}
