# JARVIS MVP — task checklist

See [plan.md](./plan.md) for full task descriptions. All `npm`/`tauri` commands assume `**cd jarvis**` first; raw `cargo` from `**cd jarvis/src-tauri**`.

## Decisions (locked)

- DB: `**rusqlite**`
- Whisper `tiny.en` model: **bundled** via Tauri resources + `scripts/download-model.ps1`, not committed
- Default global hotkey: `**Ctrl+Shift+J`**

---

## Tasks

### Task 1 — Scaffold + IPC types contract ✅ (2026-04-13)

- Create `jarvis/` via `create-tauri-app` (React, TS, Vite)
- Add frontend deps: `zustand`, `framer-motion`
- Add Rust deps: `rusqlite`, `serde`, `thiserror`
- Create `jarvis/src/types.ts` — `HudPhase`, `TranscriptUpdate`, `MatchResult`, `ActionStatus`
- Create `jarvis/src-tauri/capabilities/default.json` — `global-shortcut:allow-register`, `shell:allow-open`
- Create root `.gitignore` — `jarvis/node_modules/`, `jarvis/src-tauri/target/`, `jarvis/src-tauri/resources/*.bin`, `.env`
- **Verify:** `cd jarvis && npm run tauri dev` (window appears)
- **Verify:** `cd jarvis/src-tauri && cargo test && cargo clippy -- -D warnings`
- **Verify:** `cd jarvis && npm run build`

### Task 2 — SQLite + Action enum + CRUD + seeds ✅ (2026-04-13)

- `src-tauri/src/db/models.rs`: `CommandNode`, `Action::OpenApp`, `Action::OpenUrl` (serde round-trip)
- `src-tauri/src/db/mod.rs`: `init_db`, `insert_command`, `get_all_commands`, `get_command_by_id`, `delete_command`
- Seed on first init (empty table): "open notepad" → `OpenApp`, "open github" → `OpenUrl(https://github.com)`
- Note: `update_command` deferred to Phase 2
- **Verify:** `cargo test db::` (insert/list/get/delete + Action serde + seed idempotency)

### Task 3 — Hotkey + HUD window + click-through ✅ (2026-04-13)

- Register `Ctrl+Shift+J` global shortcut (Tauri shortcut plugin)
- Toggle `WebviewWindow`: transparent, `decorations: false`, `always_on_top: true`, skip taskbar, 480px wide, min 120px height
- `set_ignore_cursor_events(true)` when phase is `idle`/`done`/`stopped`; `false` when `listening`/`executing`
- Escape dismisses HUD (same code path as `stopped`)
- React: minimal dark glass container shell
- **Verify:** manual — hotkey from another app, Esc, click-through when idle (clicks fall through to desktop)

### Checkpoint A

- Dev app runs; hotkey toggles HUD; click-through confirmed; DB tests green; clippy clean

---

### Task 4a — Mic capture (cpal) ✅ (2026-04-13)

- `src/audio/capture.rs`: WASAPI mic stream, PCM f32 chunks → `mpsc::Sender<Vec<f32>>`
- Emit `amplitude-update` (normalized 0.0–1.0) Tauri events during capture
- `start_capture()` / `stop_capture()` — no resource leak on stop/restart
- Graceful error + event if no mic device found
- **Verify:** `cargo test audio::capture`; manual devtools — `amplitude-update` events firing

### Task 4b — Whisper inference + bundled model ✅ (2026-04-13)

- `src/audio/stt.rs`: dedicated inference thread, consumes PCM from T4a channel, runs `whisper-rs`
- Emit `transcript-update { text, is_final }` matching `src/types.ts`
- Resolve model path: `resource_dir() / "ggml-tiny.en.bin"`
- Add `bundle.resources` to `tauri.conf.json`
- Create `jarvis/scripts/download-model.ps1` (downloads to `src-tauri/resources/`)
- Create `jarvis/src-tauri/resources/.gitkeep`
- Add CMake + MSVC prereq note to `jarvis/README.md`
- Missing model → error event, no panic
- **Verify:** `cargo test audio::stt`; manual speak → `transcript-update` events in devtools

### Checkpoint B

- Mic captures audio; amplitude events visible; Whisper emits live transcript; `cargo test audio::` green

---

### Task 5 — Exact matcher ✅ (2026-04-13)

- `src/commands/matcher.rs`: case-insensitive substring match on all trigger phrases, return `Option<MatchResult>` with node + span `(start, end)`
- **Verify:** `cargo test commands::matcher` (match, no-match, multi-phrase, case, span indices)

### Task 6 — HUD UI ✅ (2026-04-13)

- `src/store/hudStore.ts`: Zustand store, subscribes to Tauri events typed against `src/types.ts`
- Transcription component: 22px centered, word-by-word stream; on match: highlight span, scale 1.0→1.05 + translateY −4px / 200ms, surrounding text fade / 300ms
- Execution phase: action status text fades in, same font/position
- Waveform circle: 44px, 7 bars 3px wide 3px gap, pulses to `amplitude-update`, flattens + fades on stop
- Stop button: 38px, square 11×11px, pulsing red border 0.4→1.0 / 1.5s cycle during `listening`, hover: full red
- `done` → 300ms pause → fade; `stopped` → 150ms fade
- Stop/Esc same code path; click-through phase changes wired to Rust
- **Verify:** manual mock-event demo cycling all 6 phases

### Task 7 — Executor ✅ (2026-04-13)

- `src/commands/executor.rs`: `OpenApp` via `cmd /C start`; `OpenUrl` via `tauri-plugin-shell`
- Emit `action-status { text }` after each action
- Validate: reject paths with shell metacharacters; only `http://` or `https://` URLs
- Error → emit event, continue remaining actions, no crash
- Add `shell:allow-open` to `capabilities/default.json`
- **Verify:** `cargo test commands::executor`; manual Notepad + GitHub URL

### Task 8 — System tray ✅ (2026-04-13)

- Tray icon + menu: Pause/Resume (label toggles), Quit
- `Arc<AtomicBool>` `is_paused` shared with `lib.rs` pipeline
- Pause blocks mic start on hotkey; Resume restores
- **Verify:** manual tray menu behavior + pause gating

### Checkpoint C

- All vertical features individually proven; `cargo test` green

---

### Task 9 — Integration

- `lib.rs` orchestrator: hotkey → `is_paused` check → show HUD + start capture → transcript → matcher (on final segments) → `match-result` → stop mic → executor → `action-status` → **4s auto-dismiss** after completion
- No-match **5s timeout** → dismiss HUD gracefully
- Stop / Esc → immediate 150ms fade out
- Tray pause gate respected
- Remove mocked events from HUD store; wire real Tauri events
- **Verify:** E2E manual checklist below

### Task 10 — Quality gates + build

- `jarvis/README.md`: prereqs, model fetch, `cd jarvis` convention, CI command sequence
- ESLint config (if not from scaffold); `npm run lint` clean
- `cargo fmt --check` + `cargo clippy -- -D warnings` clean
- **Verify:** `cd jarvis && npm run tauri build` → `.exe` produced
- **Verify:** clean install → run → E2E checklist passes

### Checkpoint D

- All SPEC Phase 1 acceptance criteria satisfied on Windows
- Human review before Phase 2

---

## Manual E2E checklist (Tasks 9–10)

1. [ ] App starts → tray icon visible, no window.
2. [ ] **Ctrl+Shift+J** → HUD appears; waveform + stop button visible, pulsing.
3. [ ] Speak "open notepad" → transcript streams in → "notepad" span highlights → Notepad opens → HUD shows "Opening notepad..." → HUD auto-dismisses after ~4s.
4. [ ] Repeat with "open github" → browser opens `https://github.com`.
5. [ ] Speak nonsense for ~5s → HUD dismisses on timeout; no crash.
6. [ ] **Ctrl+Shift+J** → listening → click Stop button → HUD fades within 150ms.
7. [ ] **Ctrl+Shift+J** → listening → press Escape → same 150ms fade (identical code path as stop).
8. [ ] Tray right-click → **Pause** → **Ctrl+Shift+J** → HUD appears but mic does NOT start.
9. [ ] Tray → **Resume** → hotkey works again normally.
10. [ ] HUD idle (not listening) → click desktop through HUD → click passes through.
11. [ ] Tray → **Quit** → process exits cleanly.

---

## Optional / post-MVP

- Vitest for Zustand reducers and pure TS helpers
- CI GitHub Actions workflow (`defaults.run.working-directory: jarvis`)
- Playwright E2E after IPC contract stable
- Restore `BrainStorm.md` to repo (needed for Phase 3 editor work)
- `update_command` DB function (needed before Phase 3 editor)