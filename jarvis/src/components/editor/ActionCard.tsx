import { useMemo } from "react";
import type { FormActionPayload } from "../../types";
import { isEditorPendingAction } from "../../types";
import { getActionKind } from "./actionCatalog";
import { EditorSelect } from "../ui/EditorSelect";
import { defaultActionForKind, type ConcreteActionKind } from "./NodeForm.logic";

type ActionCardProps = {
  action: FormActionPayload;
  index: number;
  onChange: (next: FormActionPayload) => void;
  onRemove: () => void;
};

export function ActionCard({ action, index, onChange, onRemove }: ActionCardProps) {
  const kind = getActionKind(action);
  const prefix = `action-${index}`;
  const selectValue = kind === "pending" ? "" : kind;

  const actionKindOptions = useMemo(() => {
    const opts: { value: string; label: string }[] = [
      { value: "open_app", label: "Open app" },
      { value: "open_url", label: "Open URL" },
    ];
    if (kind === "run_script") {
      opts.push({ value: "run_script", label: "Run script (legacy)" });
    }
    opts.push(
      { value: "send_keys", label: "Send keys" },
      { value: "speak", label: "Speak" },
      { value: "wait", label: "Wait" },
      { value: "sub_prompt", label: "Follow Up" },
    );
    return opts;
  }, [kind]);

  return (
    <div className="editor-action-card">
      <div className="editor-action-card-top">
        <label htmlFor={`${prefix}-action-kind`}>
          Type
          <EditorSelect
            id={`${prefix}-action-kind`}
            value={selectValue}
            placeholder="Choose action type"
            onChange={(v) => onChange(defaultActionForKind(v as ConcreteActionKind))}
            options={actionKindOptions}
          />
        </label>
        <button type="button" className="editor-delete-btn" onClick={onRemove}>
          Remove
        </button>
      </div>

      {isEditorPendingAction(action) ? null : (
        <>
          {"open_app" in action && (
            <label htmlFor={`${prefix}-app-name`}>
              App name
              <input
                id={`${prefix}-app-name`}
                value={action.open_app.name}
                onChange={(e) => onChange({ open_app: { name: e.target.value, path: "" } })}
              />
            </label>
          )}

          {"open_url" in action && (
            <label htmlFor={`${prefix}-url`}>
              URL
              <input
                id={`${prefix}-url`}
                value={action.open_url.url}
                onChange={(e) => onChange({ open_url: { url: e.target.value } })}
                placeholder="https://example.com"
              />
            </label>
          )}

          {"run_script" in action && (
            <div className="editor-form-grid-two">
              <label htmlFor={`${prefix}-script`}>
                Script
                <input
                  id={`${prefix}-script`}
                  value={action.run_script.script}
                  onChange={(e) =>
                    onChange({ run_script: { ...action.run_script, script: e.target.value } })
                  }
                />
              </label>
              <label htmlFor={`${prefix}-args`}>
                Args (comma separated)
                <input
                  id={`${prefix}-args`}
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
                />
              </label>
            </div>
          )}

          {"send_keys" in action && (
            <label htmlFor={`${prefix}-keys`}>
              Keys
              <input
                id={`${prefix}-keys`}
                value={action.send_keys.keys}
                onChange={(e) => onChange({ send_keys: { keys: e.target.value } })}
                placeholder="ctrl+shift+p"
              />
            </label>
          )}

          {"speak" in action && (
            <label htmlFor={`${prefix}-text`}>
              Text
              <input
                id={`${prefix}-text`}
                value={action.speak.text}
                onChange={(e) => onChange({ speak: { text: e.target.value } })}
              />
            </label>
          )}

          {"wait" in action && (
            <label htmlFor={`${prefix}-wait`}>
              Wait ms
              <input
                id={`${prefix}-wait`}
                type="number"
                min={0}
                value={action.wait.ms}
                onChange={(e) =>
                  onChange({
                    wait: {
                      ms: Number.isFinite(Number(e.target.value)) ? Math.max(0, Number(e.target.value)) : 0,
                    },
                  })
                }
              />
            </label>
          )}

          {"sub_prompt" in action && (
            <label htmlFor={`${prefix}-sub-prompt`}>
              Follow-up
              <input
                id={`${prefix}-sub-prompt`}
                value={action.sub_prompt.prompt}
                onChange={(e) => onChange({ sub_prompt: { prompt: e.target.value } })}
                placeholder="Follow-up question"
              />
            </label>
          )}
        </>
      )}
    </div>
  );
}
