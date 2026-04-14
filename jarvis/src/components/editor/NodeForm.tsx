import { invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useRef, useState } from "react";
import { useEditorStore } from "../../store/editorStore";
import type { CommandNodePayload } from "../../types";
import { ActionChain } from "./ActionChain";
import {
  hasBlockingErrors,
  modelFromNode,
  parseTriggerPhraseInput,
  toCommandPayload,
  validateFormModel,
  type FormModel,
} from "./NodeForm.logic";

export function NodeForm() {
  const nodes = useEditorStore((s) => s.nodes);
  const selectedId = useEditorStore((s) => s.selectedId);
  const setNodes = useEditorStore((s) => s.setNodes);
  const setSelected = useEditorStore((s) => s.setSelected);

  const selectedNode = useMemo(
    () => nodes.find((node) => node.id === selectedId) ?? null,
    [nodes, selectedId],
  );

  const [model, setModel] = useState<FormModel>(() => modelFromNode(selectedNode));
  const [triggerInput, setTriggerInput] = useState("");
  const [saving, setSaving] = useState(false);
  const [submitAttempted, setSubmitAttempted] = useState(false);
  const [toastText, setToastText] = useState<string | null>(null);
  const toastTimeoutRef = useRef<number | null>(null);

  useEffect(() => {
    setModel(modelFromNode(selectedNode));
    setTriggerInput("");
    setSubmitAttempted(false);
  }, [selectedNode]);

  useEffect(
    () => () => {
      if (toastTimeoutRef.current) {
        window.clearTimeout(toastTimeoutRef.current);
      }
    },
    [],
  );

  const errors = validateFormModel(model);
  const canSave = !saving && !hasBlockingErrors(errors);

  const updateModel = (updater: (prev: FormModel) => FormModel) => {
    setModel((prev) => updater(prev));
  };

  const showToast = (text: string) => {
    setToastText(text);
    if (toastTimeoutRef.current) {
      window.clearTimeout(toastTimeoutRef.current);
    }
    toastTimeoutRef.current = window.setTimeout(() => {
      setToastText(null);
      toastTimeoutRef.current = null;
    }, 2000);
  };

  const commitTriggerInput = () => {
    const parsed = parseTriggerPhraseInput(triggerInput);
    if (parsed.length === 0) return;
    updateModel((prev) => ({
      ...prev,
      triggerPhrases: [...prev.triggerPhrases, ...parsed],
    }));
    setTriggerInput("");
  };

  const onSave = async () => {
    setSubmitAttempted(true);
    if (!canSave) return;
    const payload = toCommandPayload(model);

    setSaving(true);
    try {
      const saved = model.id
        ? await invoke<CommandNodePayload>("update_command", { id: model.id, node: payload })
        : await invoke<CommandNodePayload>("create_command", { node: payload });

      const latestNodes = useEditorStore.getState().nodes;
      const existingIndex = latestNodes.findIndex((entry) => entry.id === saved.id);
      const nextNodes =
        existingIndex === -1
          ? [...latestNodes, saved]
          : latestNodes.map((entry) => (entry.id === saved.id ? saved : entry));
      setNodes(nextNodes);
      setSelected(saved.id);
      setModel(modelFromNode(saved));
      showToast("Saved");
      setSubmitAttempted(false);
    } catch (err) {
      showToast(`Save failed: ${String(err)}`);
    } finally {
      setSaving(false);
    }
  };

  const onCancel = () => {
    setModel(modelFromNode(selectedNode));
    setTriggerInput("");
    setSubmitAttempted(false);
  };

  const showErrors = submitAttempted;

  return (
    <section className="editor-panel editor-panel-right">
      <header className="editor-panel-header">
        <h2>{model.id ? `Edit: ${selectedNode?.name ?? "Node"}` : "New Node"}</h2>
      </header>

      <div className="editor-form-scroll">
        {toastText && (
          <div className="editor-inline-toast" role="status">
            {toastText}
          </div>
        )}

        <div className="editor-form-grid">
          <label>
            Name
            <input
              value={model.name}
              onChange={(e) => updateModel((prev) => ({ ...prev, name: e.target.value }))}
              placeholder="Open calculator"
            />
          </label>
          {showErrors && errors.name && <p className="editor-field-error">{errors.name}</p>}

          <label>
            Trigger phrases
            <div className="editor-trigger-row">
              <input
                value={triggerInput}
                onChange={(e) => setTriggerInput(e.target.value)}
                onBlur={commitTriggerInput}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === ",") {
                    e.preventDefault();
                    commitTriggerInput();
                  }
                }}
                placeholder="type phrase, press enter"
              />
              <button type="button" onClick={commitTriggerInput}>
                Add
              </button>
            </div>
          </label>
          {model.triggerPhrases.length > 0 && (
            <div className="editor-tag-list">
              {model.triggerPhrases.map((phrase, idx) => (
                <button
                  key={`${phrase}-${idx}`}
                  type="button"
                  className="editor-tag"
                  onClick={() =>
                    updateModel((prev) => ({
                      ...prev,
                      triggerPhrases: prev.triggerPhrases.filter((_, i) => i !== idx),
                    }))
                  }
                  title="Remove trigger phrase"
                >
                  {phrase} ×
                </button>
              ))}
            </div>
          )}
          {showErrors && errors.triggerPhrases && <p className="editor-field-error">{errors.triggerPhrases}</p>}

          <label>
            Fuzzy threshold: {model.threshold.toFixed(2)}
            <input
              type="range"
              min={0.5}
              max={1}
              step={0.01}
              value={model.threshold}
              onChange={(e) => updateModel((prev) => ({ ...prev, threshold: Number(e.target.value) }))}
            />
          </label>
          {showErrors && errors.threshold && <p className="editor-field-error">{errors.threshold}</p>}

          <label className="editor-checkbox-row">
            <input
              type="checkbox"
              checked={model.enabled}
              onChange={(e) => updateModel((prev) => ({ ...prev, enabled: e.target.checked }))}
            />
            Enabled
          </label>
        </div>

        <ActionChain
          title="Action Chain"
          actions={model.actions}
          onChange={(actions) => updateModel((prev) => ({ ...prev, actions }))}
          errorByIndex={showErrors ? errors.actionUrls : {}}
        />
        {showErrors && errors.actions && <p className="editor-field-error">{errors.actions}</p>}

        <section className="editor-subprompt-panel">
          <h3>Sub-prompt</h3>
          <label>
            Prompt text
            <input
              value={model.subPromptText}
              onChange={(e) => updateModel((prev) => ({ ...prev, subPromptText: e.target.value }))}
              placeholder="What should I search?"
            />
          </label>
          {showErrors && errors.subPromptText && <p className="editor-field-error">{errors.subPromptText}</p>}

          <ActionChain
            title="Sub-prompt Action Chain"
            actions={model.subPromptActions}
            onChange={(subPromptActions) => updateModel((prev) => ({ ...prev, subPromptActions }))}
            errorByIndex={showErrors ? errors.subPromptUrls : {}}
          />
        </section>
      </div>

      <footer className="editor-form-actions">
        <button type="button" onClick={onCancel}>
          Cancel
        </button>
        <button type="button" onClick={onSave} disabled={!canSave}>
          {saving ? "Saving..." : "Save"}
        </button>
      </footer>
    </section>
  );
}
