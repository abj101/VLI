import type { ReactNode } from "react";
import { useId } from "react";

type SettingsLabelWithInfoProps = {
  children: ReactNode;
  tip: string;
  /** Id for aria-describedby on related controls. */
  tipId?: string;
  /** Optional id for aria-labelledby on related controls. */
  id?: string;
};

/** Inline setting label; hover shows help tooltip. */
export function SettingsLabelWithInfo({
  children,
  tip,
  tipId: tipIdProp,
  id,
}: SettingsLabelWithInfoProps) {
  const reactId = useId();
  const tipId = tipIdProp ?? `settings-tip-${reactId.replace(/:/g, "")}`;

  return (
    <span id={id} className="editor-settings-label-with-tip">
      {children}
      <span id={tipId} role="tooltip" className="editor-settings-info-tip-popover">
        {tip}
      </span>
    </span>
  );
}
