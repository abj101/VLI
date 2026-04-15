/** Settings panes used by the editor shell sidebar and the settings modal. */
export type EditorSettingsNavId = "hotkeys" | "recognition" | "appearance" | "about";

export const EDITOR_SETTINGS_NAV: { id: EditorSettingsNavId; label: string }[] = [
  { id: "hotkeys", label: "Hotkeys" },
  { id: "recognition", label: "Recognition" },
  { id: "appearance", label: "Appearance" },
  { id: "about", label: "About" },
];
