import type { RefObject } from "react";
import { EditorCheckIcon } from "./EditorCheckIcon";
import { EditorCloseXIcon } from "./EditorCloseXIcon";

export type CommandDeleteConfirmProps = {
  deletePending: boolean;
  phraseLabel: string;
  onRequestDelete: () => void;
  onCancel: () => void;
  onConfirm: () => void;
  cancelRef?: RefObject<HTMLButtonElement | null>;
};

export function CommandDeleteConfirm({
  deletePending,
  phraseLabel,
  onRequestDelete,
  onCancel,
  onConfirm,
  cancelRef,
}: CommandDeleteConfirmProps) {
  const label = phraseLabel.trim() || "command";

  return (
    <div
      className="editor-command-delete-confirm"
      data-pending={deletePending ? "true" : "false"}
    >
      <div
        className="editor-command-delete-confirm__idle"
        aria-hidden={deletePending}
      >
        <button
          type="button"
          className="editor-command-delete"
          onClick={onRequestDelete}
          aria-label={`Delete ${label}`}
          tabIndex={deletePending ? -1 : 0}
        >
          <EditorCloseXIcon className="editor-command-delete-x" />
        </button>
      </div>
      <div
        className="editor-command-delete-confirm__pending"
        role="group"
        aria-label="Confirm delete"
        aria-hidden={!deletePending}
      >
        <button
          ref={cancelRef}
          type="button"
          className="editor-command-draft-icon-btn"
          onClick={onCancel}
          aria-label="Cancel delete"
          tabIndex={deletePending ? 0 : -1}
        >
          <span className="editor-command-draft-icon" aria-hidden>
            <EditorCloseXIcon className="editor-command-draft-icon-svg" />
          </span>
        </button>
        <button
          type="button"
          className="editor-command-draft-icon-btn editor-command-draft-icon-btn--confirm"
          onClick={onConfirm}
          aria-label="Confirm delete"
          tabIndex={deletePending ? 0 : -1}
        >
          <span className="editor-command-draft-icon" aria-hidden>
            <EditorCheckIcon className="editor-command-draft-icon-svg" />
          </span>
        </button>
      </div>
    </div>
  );
}
