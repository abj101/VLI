# Phase 3 Todo — JARVIS React Command Editor

> Tick each item only after the stated verification passes. Do not skip items to unblock a later task — resolve or explicitly defer with a note.

---

## Task 3-1: Editor Window + Tray Entry Point

- [x] Add `"Open Editor"` item to tray menu above Pause/Resume (`tray.rs`)
- [x] Implement `open_editor` Tauri command in `lib.rs` with `get_webview_window("editor").is_some()` guard
- [x] Define editor window in `tauri.conf.json` (decorated, resizable, 900×600 min, centered)
- [x] Add `editorMain.tsx` as second Vite entry point
- [x] Create `EditorRoot.tsx` placeholder (`<div>Editor coming soon</div>`)
- [x] Update `vite.config.ts` for multi-entry build
- [ ] **Verify:** Tray → "Open Editor" → window appears
- [ ] **Verify:** Click tray again while open → focuses existing window (no duplicate)
- [ ] **Verify:** HUD hotkey still fires and HUD appears while editor is open
- [x] **Verify:** `cargo clippy -- -D warnings` clean

---

## Task 3-2: IPC — `update_command` + Full CRUD Surface

- [x] Implement `update_command(id, node)` in `db/mod.rs`
- [x] Add `cargo test db::update` — round-trip update test
- [x] Register all 5 IPC commands in `lib.rs`: `list_commands`, `get_command`, `create_command`, `update_command`, `delete_command`
- [x] Add Rust-side payload validation (empty name, zero trigger phrases, threshold out of range → `Err(String)`)
- [x] Add `CommandNodePayload` and `ActionPayload` types to `src/types.ts` if not already present
- [x] **Verify:** `cd jarvis/src-tauri && cargo test db::`  — all green including new update test
- [ ] **Verify:** Manual devtools `invoke("update_command", {...})` returns success *(deferred: requires interactive app/devtools session)*
- [x] **Verify:** Invalid payloads return error strings (not panics)

---

## Task 3-3: NodeList Panel

- [ ] Create `editorStore.ts` (Zustand): `nodes`, `selectedId`, `setSelected`, `setNodes`, `deleteNode`, `toggleEnabled`
- [ ] Write Vitest unit tests for `editorStore` (`NodeList.test.ts`)
- [ ] Create `NodeList.tsx`: loads via `invoke("list_commands")` on mount
- [ ] Render each row: name, first trigger phrase, enabled toggle (pill switch), delete button
- [ ] Enabled toggle: optimistic update + `invoke("update_command")` + revert on error with inline toast
- [ ] Delete: `confirm()` prompt + `invoke("delete_command")` + remove from store
- [ ] Empty state: centered message + large `+` button
- [ ] `+` header button: clears `editorStore.selectedId` (signals NodeForm to show blank form)
- [ ] Wire `NodeList` into `EditorRoot.tsx` left panel
- [ ] **Verify:** Seeded nodes appear on first editor open
- [ ] **Verify:** Delete node → close/reopen editor → gone
- [ ] **Verify:** Toggle enabled → close/reopen → toggle state persisted
- [ ] **Verify:** `cd jarvis && npm run test` — `editorStore` tests pass
- [ ] **Verify:** `npm run lint` clean

---

## Task 3-4: NodeForm + ActionChain Editor

- [ ] **Confirm `@dnd-kit/core` dep addition before installing** (ask human)
- [ ] Install `@dnd-kit/core` (after confirmation)
- [ ] Create `ActionCard.tsx`: type selector dropdown + type-specific fields for all 6 action types (`open_app`, `open_url`, `run_script`, `send_keys`, `speak`, `wait`)
- [ ] Create `ActionChain.tsx`: ordered list of `ActionCard` rows; add (`+`) / remove (`×`) per row; drag-and-drop reorder via `@dnd-kit`; up/down arrow fallback
- [ ] Create `NodeForm.tsx` with fields: name, trigger phrase tag-input, fuzzy threshold slider (0.50–1.00, step 0.01), enabled toggle, ActionChain, sub_prompt section (text + nested ActionChain depth=1), Save / Cancel buttons
- [ ] Form validation (inline field errors): name required, ≥1 trigger phrase, ≥1 action, valid URL for `open_url`, threshold in range — block Save until valid
- [ ] Write `NodeForm.test.ts` Vitest validation unit tests
- [ ] Save: `create_command` (new node) or `update_command` (existing); on success update `editorStore.nodes`, show 2s inline toast
- [ ] Selecting a row in NodeList populates form; "New" clears form
- [ ] Wire `NodeForm` into `EditorRoot.tsx` right panel (two-panel layout)
- [ ] **Verify:** Create "open calculator" node → save → fire hotkey → Calculator opens (no restart)
- [ ] **Verify:** Edit trigger phrase → save → new phrase matches immediately
- [ ] **Verify:** Drag action rows → save → reopen editor → order preserved
- [ ] **Verify:** `sub_prompt` saves and reloads correctly
- [ ] **Verify:** `cargo test db::` green (action JSON round-trip still intact)
- [ ] **Verify:** `npm run test` — NodeForm validation tests pass
- [ ] **Verify:** `npm run lint` clean

---

## Checkpoint A — Editor Foundation ✋ HUMAN SIGN-OFF REQUIRED

Before continuing to T3-5 / T3-6, confirm all of the following:

- [ ] Editor window opens from tray; HUD unaffected while editor open
- [ ] NodeList loads, deletes, toggles nodes from live SQLite
- [ ] NodeForm creates and edits all 6 action types with inline validation
- [ ] `update_command` IPC fully tested (`cargo test db::`)
- [ ] `npm run lint` clean
- [ ] `cargo clippy -- -D warnings` clean
- [ ] **Human has reviewed and approved** — do not proceed until this is checked

---

## Task 3-5: Settings Panel

- [ ] Add `settings` table migration: `CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)` — runs on startup, idempotent
- [ ] Create `db/settings.rs`: `get_setting(key)`, `set_setting(key, value)` helpers
- [ ] Add `cargo test db::settings` — round-trip insert/update/get
- [ ] Register `get_setting`, `set_setting`, `set_hotkey` IPC commands in `lib.rs`
- [ ] `set_hotkey`: deregister old → register new → persist to settings table → return error if combo unregisterable (do not leave hotkey deregistered on failure)
- [ ] Create `SettingsPanel.tsx` with three sections: Hotkey, Default fuzzy threshold, Theme
- [ ] Theme selector applies CSS class to `<html>` in editor window immediately; persists to settings
- [ ] Add gear icon in editor header that toggles settings panel
- [ ] **Verify:** Change hotkey → close app → reopen → new hotkey works
- [ ] **Verify:** Invalid hotkey (empty) → inline error, old hotkey still active
- [ ] **Verify:** Light/dark theme toggle → immediate re-render → persists on reopen
- [ ] **Verify:** Default threshold change → persists → used by matcher when node has no override
- [ ] **Verify:** `cargo test db::settings`

---

## Task 3-6: Drag-and-Drop Node Reorder in NodeList

- [ ] Write `MIGRATIONS.md` at `jarvis/src-tauri/` documenting migration log format
- [ ] Add `sort_order INTEGER DEFAULT 0` column: `ALTER TABLE command_nodes ADD COLUMN sort_order INTEGER DEFAULT 0` — idempotent (check column exists first)
- [ ] Update `list_commands` to `ORDER BY sort_order ASC, id ASC`
- [ ] Implement `reorder_commands(ordered_ids: Vec<i64>)` in `db/mod.rs` — bulk update `sort_order`
- [ ] Add `cargo test db::reorder`
- [ ] Register `reorder_commands` IPC command in `lib.rs`
- [ ] Add drag handles (⠿) to `NodeList.tsx` rows — visible on hover, `@dnd-kit` integration
- [ ] Up/down arrow button fallback per row (keyboard accessible)
- [ ] On drag end: call `invoke("reorder_commands", { orderedIds: [...] })`; optimistic store update
- [ ] **Verify:** Drag nodes → close editor → reopen → order preserved
- [ ] **Verify:** Pipeline still matches commands correctly after reorder (sort_order does not affect matching)
- [ ] **Verify:** `cargo test db::reorder`
- [ ] **Verify:** Migration runs cleanly on a pre-Phase-3 DB file (test with a copied Phase 2 DB)

---

## Checkpoint B — Full Editor Feature-Complete ✋ HUMAN SIGN-OFF REQUIRED

Before T3-7, confirm all of the following:

- [ ] NodeList: renders, reorders (drag + arrow), toggles, deletes
- [ ] NodeForm: creates, edits, all 6 action types, sub_prompt, drag-drop action chain
- [ ] Settings: hotkey, default threshold, theme — all persist across restart
- [ ] `MIGRATIONS.md` exists and documents the sort_order migration
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `npm run lint` clean
- [ ] `npm run test` clean
- [ ] **Human has reviewed and approved** — do not proceed until this is checked

---

## Task 3-7: Integration — Live Pipeline Reload + ai_mode Preview

- [ ] Introduce `CommandCache` in `lib.rs`: `Arc<RwLock<Vec<CommandNode>>>` loaded from DB on startup
- [ ] Matcher reads from cache (read lock) instead of querying DB per transcript segment
- [ ] Each editor IPC write (create, update, delete, reorder) refreshes cache after DB write (write lock)
- [ ] Invalidate `editorStore` / trigger refresh in React after save (so NodeList stays in sync)
- [ ] Implement `ai_mode` branch in `executor.rs`: after actions run, if `node.ai_mode == true` AND `ANTHROPIC_API_KEY` env var is set → call `claude-haiku-4-5` with `sub_prompt` text → emit result as `transcript-update { text: <response>, is_final: true }`
- [ ] **Confirm `reqwest` or Anthropic crate addition before adding dep**
- [ ] If `ai_mode == true` but no key → log warning, skip silently, no panic
- [ ] If `ai_mode == false` → no API call regardless of key presence
- [ ] API key never logged, never emitted over IPC events
- [ ] **Verify:** Create node in editor → immediately fire hotkey (no restart) → node executes
- [ ] **Verify:** Delete node in editor → immediately fire hotkey → no match
- [ ] **Verify:** `ai_mode: true` node + `ANTHROPIC_API_KEY` set → HUD shows AI reply
- [ ] **Verify:** `ai_mode: true` node + no key → no crash, warning in log only
- [ ] **Verify:** `cargo test` full suite still green

---

## Task 3-8: Quality Gates + README + BrainStorm Restore

- [ ] **Restore `BrainStorm.md`** at repo root from git history or original spec session
- [ ] Confirm `BrainStorm.md` contains Phase 4 editor spec content (wake-word, AI mode UX)
- [ ] Add Phase 3 section to `jarvis/README.md`: opening editor, editor keyboard shortcuts, settings location, migration note
- [ ] Add Vitest coverage threshold config: **70%+** line coverage on `editorStore`, NodeForm validation, ActionChain logic
- [ ] `cd jarvis && npm run lint` — clean
- [ ] `cd jarvis && npm run test` — green, coverage ≥70% on editor modules
- [ ] `cd jarvis/src-tauri && cargo fmt --check` — clean
- [ ] `cd jarvis/src-tauri && cargo clippy -- -D warnings` — clean
- [ ] `cd jarvis && npm run tauri build` — produces `.exe`
- [ ] Install `.exe` on clean Windows machine (no dev tools) → open editor from tray → create command → trigger via hotkey → executes

---

## Checkpoint C — Phase 3 Complete ✋ HUMAN SIGN-OFF REQUIRED

- [ ] All tasks T3-1 through T3-8 checked off
- [ ] `BrainStorm.md` restored at repo root
- [ ] Full E2E checklist on clean install:
  - [ ] App launches; tray icon appears
  - [ ] Hotkey fires; HUD appears; speech transcribes
  - [ ] Seeded command matches and executes
  - [ ] Editor opens from tray; no duplicate window on second click
  - [ ] Create new command in editor; trigger immediately via hotkey (no restart)
  - [ ] Edit trigger phrase; trigger new phrase immediately
  - [ ] Delete command; verify it no longer matches
  - [ ] Reorder nodes; verify order persists
  - [ ] Change hotkey in settings; new hotkey works after change
  - [ ] Tray pause → hotkey has no effect; resume → hotkey works again
  - [ ] Quit via tray → process exits cleanly
- [ ] All quality gates green: lint, test (≥70% editor coverage), fmt, clippy, build
- [ ] **Human has reviewed and approved Phase 3** — do not begin Phase 4 until this is checked

---

## Phase 4 Preview (out of scope — for planning reference only)

Once Phase 3 is signed off, Phase 4 will cover:
- Porcupine or OpenWakeWord wake-word (decision in Open Questions)
- Full `ai_mode` settings UI (API key entry, model selector)
- App auto-detection for `open_app` (enumerate running / installed apps)

Restore and review `BrainStorm.md` before starting Phase 4.