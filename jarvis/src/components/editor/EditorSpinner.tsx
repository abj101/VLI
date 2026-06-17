type EditorSpinnerProps = {
  /** Diameter in px (default 14). */
  size?: number;
  className?: string;
};

/** Circular ring spinner shared by Settings and the command composer. */
export function EditorSpinner({ size = 14, className = "" }: EditorSpinnerProps) {
  const classes = ["editor-spinner", className].filter(Boolean).join(" ");
  return (
    <span
      className={classes}
      style={{ width: size, height: size }}
      aria-hidden
    />
  );
}
