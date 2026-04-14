# Phase 4 — Ordered task list (with acceptance + verification)

Work in order unless noted **(parallel)**.

---

## Task 1: Wake backends — close gaps

**Description:** Confirm Porcupine + OWW match `references/Phase4Todo.md` T4-1/T4-2. Wire `oww_threshold` from settings when wake thread exists (T4-5).

**Acceptance criteria:**

- All T4-1/T4-2 items in Phase4Todo either done or explicitly deferred to Task 2 with a comment in code/issues.
- Default build without `oww` has no OWW symbols.

**Verification:**

- `cargo test audio::wake::`
- `cargo test --features oww audio::wake::oww` (if `oww` enabled)

**Dependencies:** None.

**Files likely touched:** `jarvis/src-tauri/src/audio/wake/`*, `db/settings.rs`.

**Estimated scope:** Small–medium.

---

## Task 2: Transcription backend abstraction (T4-3)

**Description:** Introduce provider selection: **local** (bundled), **OS**, **remote API**. Persist choice in settings. Implement **local** path to current behavior; OS/remote may stub with clear errors until implemented.

**Acceptance criteria:**

- Three provider classes represented in types + persisted settings.
- Selecting local uses existing pipeline; transcript still feeds matcher unchanged.
- No LLM HTTP client added for command interpretation.

**Verification:**

- `cargo test` for new transcription module(s).
- Manual: cycle provider in Settings (after Task 3 UI) and confirm local works.

**Dependencies:** Task 1 optional (can parallel).

**Files likely touched:** `jarvis/src-tauri/src/audio/`*, `lib.rs`, `db/settings.rs`.

**Estimated scope:** Medium.

---

## Task 3: Settings IPC + UI for wake + STT (T4-4)

**Description:** Expose STT fields over Tauri commands; update `SettingsPanel` / `settingsStore`. Remove or hide Anthropic + global AI mode when Task 5 executes (same PR acceptable).

**Acceptance criteria:**

- User can see and save wake engine + STT provider + remote fields as defined in Task 2.
- Secrets not stored in SQLite.

**Verification:**

- `cargo test db::settings`
- Manual: Settings round-trip after restart.

**Dependencies:** Task 2 (types + keys).

**Files likely touched:** `jarvis/src-tauri/src/lib.rs`, `db/settings.rs`, `SettingsPanel.tsx`, `settingsStore.ts`.

**Estimated scope:** Medium.

---

## Task 4: Wake path integration (T4-5)

**Description:** Spawn wake thread when enabled; emit `wake-detected`; gate on pause; secondary PCM feed.

**Acceptance criteria:**

- Matches Phase4Todo T4-5 checklist.
- Hotkey-only mode unchanged when `wake_engine = hotkey`.

**Verification:**

- Manual: speak wake word → pipeline starts.
- `cargo test` full suite.

**Dependencies:** Task 3 (read settings).

**Files likely touched:** `lib.rs`, `audio/mod.rs`.

**Estimated scope:** Medium.

---

## Task 5: Legacy AI removal — Haiku, `ai_mode`, `ai` module (T4-6)

**Description:** Delete LLM command path: `jarvis/src-tauri/src/ai/`, `ai_mode` on nodes, Anthropic key + `global_ai_mode` in settings, executor preview calls, related TS types/editor logic, tests. Add migrations as needed.

**Acceptance criteria:**

- No `run_ai_mode` / Haiku / `claude-haiku` in production code paths.
- DB and UI consistent; editor saves valid commands without `ai_mode`.
- Full test suite green.

**Verification:**

- `rg` / search in repo for `anthropic`, `ai_mode`, `claude-haiku` (allow only docs/changelog if any).
- `cargo clippy -- -D warnings`, `npm run lint`, `npm run test`.

**Dependencies:** Task 2–3 recommended so STT UI replaces removed panels.

**Files likely touched:** `ai/mod.rs` (delete), `executor.rs`, `db/`*, `lib.rs`, `keychain.rs`, `SettingsPanel.tsx`, `NodeForm.logic.ts`, `types.ts`, tests.

**Estimated scope:** Medium–large.

---

## Task 6: App index (T4-7)

**Description:** Windows scanner + SQLite cache + fuzzy resolve + executor hook.

**Acceptance criteria:**

- Phase4Todo T4-7 checklist satisfied.
- `app-index-ready` emitted.

**Verification:**

- `cargo test apps::`
- Manual: Settings shows count > 0 on dev PC.

**Dependencies:** Phase 3 DB (parallel with Task 4–5 if no schema conflict).

**Files likely touched:** `apps/`*, `executor.rs`, `db/mod.rs`.

**Estimated scope:** Medium.

---

## Task 7: End-to-end integration (T4-8)

**Description:** Startup order, HUD wake badge, tray tooltip, no LLM thinking UI, degradation scenarios.

**Acceptance criteria:**

- Phase4Todo T4-8 satisfied (minus LLM-specific bullets).
- Phase 1–3 regression checklist passes.

**Verification:**

- Manual E2E per Phase4Todo matrix (updated).
- `npm run build`.

**Dependencies:** Tasks 4–6.

**Estimated scope:** Medium.

---

## Task 8: Quality + docs (T4-9)

**Description:** fmt, clippy, eslint, vitest, tauri build; README Phase 4 for wake + STT + app index.

**Acceptance criteria:**

- All gates in Phase4Todo T4-9 pass.
- README does not document Anthropic for commands.

**Verification:**

- Commands listed in Phase4Todo T4-9.

**Dependencies:** Task 7.

**Estimated scope:** Small.

---

## Checkpoint: After Tasks 2–3

- STT settings persist; local transcription works.
- No new dependencies on Anthropic for new work.

## Checkpoint: After Tasks 5–6

- Legacy AI code gone or inert.
- App index integrated.

## Checkpoint: After Task 8

- Ready for human sign-off and Phase 5 planning.