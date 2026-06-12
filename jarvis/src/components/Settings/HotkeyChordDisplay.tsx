import {
  formatHotkeyKeyLabel,
  isHotkeyModifierToken,
  parseHotkeyChord,
} from "../editor/SettingsPanel.logic";

type HotkeyChordDisplayProps = {
  chord: string;
  recording: boolean;
  placeholder?: string;
};

export function HotkeyChordDisplay({
  chord,
  recording,
  placeholder = "ctrl+shift+j",
}: HotkeyChordDisplayProps) {
  const tokens = parseHotkeyChord(chord);

  if (tokens.length === 0) {
    return (
      <span
        className={`editor-hotkey-chord editor-hotkey-chord--empty${recording ? " editor-hotkey-chord--recording" : ""}`}
        role="status"
        aria-live="polite"
      >
        {recording ? "Press shortcut…" : placeholder}
      </span>
    );
  }

  return (
    <span
      className={`editor-hotkey-chord${recording ? " editor-hotkey-chord--recording" : ""}`}
      role="status"
      aria-live="polite"
      aria-label={tokens.map(formatHotkeyKeyLabel).join(" plus ")}
    >
      {tokens.map((token, index) => (
        <span key={`${token}-${index}`} className="editor-hotkey-chord-segment">
          {index > 0 && (
            <span className="editor-hotkey-chord-plus" aria-hidden>
              +
            </span>
          )}
          <kbd
            className={`editor-hotkey-key${isHotkeyModifierToken(token) ? " editor-hotkey-key--modifier" : ""}`}
          >
            {formatHotkeyKeyLabel(token)}
          </kbd>
        </span>
      ))}
    </span>
  );
}
