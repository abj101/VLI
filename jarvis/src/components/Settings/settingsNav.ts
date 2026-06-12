/** Settings panes used by the editor shell sidebar and the settings modal. */
export type EditorSettingsNavId = "hotkeys" | "recognition" | "appearance" | "app-index";

export const EDITOR_SETTINGS_NAV: { id: EditorSettingsNavId; label: string }[] = [
  { id: "hotkeys", label: "Hotkeys" },
  { id: "recognition", label: "Recognition" },
  { id: "appearance", label: "Appearance" },
  { id: "app-index", label: "App Index" },
];
