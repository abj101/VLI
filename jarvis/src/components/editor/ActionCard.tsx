import type { ActionPayload } from "../../types";
import { defaultActionForKind } from "./NodeForm.logic";

type ActionCardProps = {
  action: ActionPayload;
  index: number;
  onChange: (next: ActionPayload) => void;
  onRemove: () => void;
};

export function ActionCard({ action, index, onChange, onRemove }: ActionCardProps) {
  const kind = getActionKind(action);
  const prefix = `action-${index}`;

  return (
    <div className="editor-action-card">
      <div className="editor-action-card-top">
        <label>
          Type
          <select
            value={kind}
            onChange={(e) => onChange(defaultActionForKind(e.target.value as ReturnType<typeof getActionKind>))}
          >
            <option value="open_app">Open app</option>
            <option value="open_url">Open URL</option>
            <option value="run_script">Run script</option>
            <option value="send_keys">Send keys</option>
            <option value="speak">Speak</option>
            <option value="wait">Wait</option>
          </select>
        </label>
        <button type="button" className="editor-delete-btn" onClick={onRemove}>
          Remove
        </button>
      </div>

      {"open_app" in action && (
        <div className="editor-form-grid-two">
          <label htmlFor={`${prefix}-app-name`}>
            App name
            <input
              id={`${prefix}-app-name`}
              value={action.open_app.name}
              onChange={(e) => onChange({ open_app: { ...action.open_app, name: e.target.value } })}
            />
          </label>
          <label htmlFor={`${prefix}-app-path`}>
            App path
            <input
              id={`${prefix}-app-path`}
              value={action.open_app.path}
              onChange={(e) => onChange({ open_app: { ...action.open_app, path: e.target.value } })}
            />
          </label>
        </div>
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
    </div>
  );
}

function getActionKind(action: ActionPayload) {
  if ("open_app" in action) return "open_app" as const;
  if ("open_url" in action) return "open_url" as const;
  if ("run_script" in action) return "run_script" as const;
  if ("send_keys" in action) return "send_keys" as const;
  if ("speak" in action) return "speak" as const;
  return "wait" as const;
}
