# JARVIS — BrainStorm design notes

Living reference for HUD visuals, command editor UX, and Phase 4+ roadmap. Recovered from Phase 1 `references/plan.md` (Task 6 inline HUD spec), `references/Phase3Plan.md`, and product checkpoints. **No emoji in editor UI**; geometric icons; shared **10px corner radius** and **44px** row height where applicable.

---

## HUD overlay (Phase 1)

### Transcript

- **Font:** 22px, centered, near-white.
- **Streaming:** word-by-word updates from Whisper partial/final segments.
- **On match:** matched span gets background highlight + scale **1.0 → 1.05** + **translateY −4px** over **200ms** ease-out. Surrounding text fades opacity → 0 over **300ms**. Then matched text fades.
- **Execution:** action status text fades in at same font/position; no icons.

### Waveform circle

- **Placement:** bottom-left.
- **Size:** **44px** diameter.
- **Bars:** 7 bars, **3px** wide, **3px** gap, centered.
- **Behavior:** pulses to `amplitude-update` (0..1) during `listening`; bars animate flat then circle fades on stop.

### Stop control

- **Placement:** bottom-right.
- **Size:** **38px** diameter; inner square **11×11px** centered.
- **Resting:** muted red fill + red border.
- **Listening:** border pulses opacity **0.4 → 1.0**, **1.5s** cycle.
- **Hover:** full red fill; no pulse.
- **Label:** **9px** mono under icon (e.g. `Esc`).
- **Action:** click = same path as **Escape** → `stopped` transition.

### Auto-dismiss

- `done` → **300ms** pause → panel fades.
- `stopped` → **150ms** fade.

### Click-through

- HUD notifies Rust (`set_ignore_cursor_events`) when phase crosses to/from **idle** so the desktop stays clickable when the overlay is inactive.

### Phases

`idle | listening | matched | executing | awaiting_input | done | stopped` — transitions and animations should align with `HudPhase` in `jarvis/src/types.ts`.

---

## Command editor (Phase 3)

### Shell

- Separate `**WebviewWindow` label `"editor"`** — not the HUD. Decorated, resizable, taskbar-visible; **900×600** minimum; single instance (focus if already open).

### Layout

- **Left:** `NodeList` — command nodes from SQLite (`list_commands`), selection, enable toggle, delete, reorder (drag handle + keyboard arrows), empty state, “new command”.
- **Right:** `NodeForm` — name, trigger phrase tags, fuzzy threshold (default **0.80** SPEC), enabled, **action chain** (all action kinds), optional **sub_prompt** text + nested chain (depth 1).
- **Header:** gear opens **Settings** (hotkey, default fuzzy threshold, theme).

### Persistence

- Commands: CRUD + `reorder_commands` + `sort_order` column (see `jarvis/src-tauri/MIGRATIONS.md`).
- Settings: `settings` key/value table (hotkey string, theme, defaults).

### Pipeline integration

- In-memory **command cache** refreshed on editor writes so matching executes **without app restart**.
- `**ai_mode`:** when `true` and `ANTHROPIC_API_KEY` is set, optional Haiku follow-up using `sub_prompt`; never log or emit the API key.

---

## Phase 4 preview (out of scope until Phase 3 sign-off)


| Area             | Direction                                                                                                                  |
| ---------------- | -------------------------------------------------------------------------------------------------------------------------- |
| **Wake word**    | Porcupine or OpenWakeWord — decision in open questions; runs alongside or instead of push-to-talk hotkey where configured. |
| `**ai_mode` UX** | Settings UI for API key and model selection (env-only in Phase 3 is OK for devs).                                          |
| `**open_app`**   | Auto-detect installed/running apps to reduce manual names/paths.                                                           |
| **Polish**       | Confirm editor measurements against this doc when implementing new panels.                                                 |


---

## Release / quality gates

Before shipping editor-heavy builds:

- `npm run lint`, `npm run test` (with editor module coverage thresholds in `jarvis/vite.config.ts`).
- `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`.
- `npm run tauri build` after `jarvis/scripts/download-model.ps1` so Whisper weights bundle.

---

## References

- `references/plan.md` — Phase 1 tasks and deferred items.
- `references/Phase3Plan.md` — Phase 3 architecture and checkpoints.
- `jarvis/README.md` — build prerequisites and Phase 3 usage.