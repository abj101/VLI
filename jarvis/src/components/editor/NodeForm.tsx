import { invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useRef, useState } from "react";
import { formatUserError } from "../../utils/userErrors";
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
import { getPrimaryTriggerPhrase } from "./NodeList.logic";
import { EditorCloseXIcon } from "./EditorCloseXIcon";

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

      const refreshed = await invoke<CommandNodePayload[]>("list_commands");
      setNodes(refreshed);
      setSelected(saved.id);
      setModel(modelFromNode(saved));
      showToast("Saved");
      setSubmitAttempted(false);
    } catch (err) {
      showToast(formatUserError(err, "Could not save this command."));
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
    <section className="editor-panel editor-glass-panel editor-panel-right">
      <header className="editor-panel-header">
        <h2>
          {model.id && selectedNode ? `Edit: ${getPrimaryTriggerPhrase(selectedNode)}` : "New Node"}
        </h2>
      </header>

      <div className="editor-form-scroll">
        {toastText && (
          <div className="editor-inline-toast" role="status">
            {toastText}
          </div>
        )}

        <div className="editor-form-grid">
          <label>
            Trigger phrases
            <div className="editor-trigger-row">
              <input
                value={triggerInput}
                onChange={(e) => setTriggerInput(e.target.value)}
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
                  <span className="editor-tag-text">{phrase}</span>
                  <EditorCloseXIcon className="editor-tag-close-x" />
                </button>
              ))}
            </div>
          )}
          {showErrors && errors.triggerPhrases && <p className="editor-field-error">{errors.triggerPhrases}</p>}

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
          errorByIndex={showErrors ? errors.actionErrors : {}}
        />
        {showErrors && errors.actions && <p className="editor-field-error">{errors.actions}</p>}
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
