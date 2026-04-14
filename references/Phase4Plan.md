# Implementation Plan: JARVIS Phase 4

## Scope

Phase 4 adds three capabilities that were explicitly deferred in Phase 1–3:

1. **Wake-word engine** — Porcupine (primary) or OpenWakeWord (fallback / custom phrase) replaces the hotkey-only trigger for always-on listening.
2. **Haiku `ai_mode`** — When a command node has `ai_mode: true`, an Anthropic Claude Haiku call is made using the user-supplied API key for intent extraction / open-ended replies; no hardcoded cloud by default.
3. **App auto-detection** — `OpenApp` actions resolve against a live index of installed applications (Windows registry / Start Menu scan) instead of requiring the user to supply a raw executable path.

Phases 1–3 are complete and all checklists have passed. This plan builds on that foundation; no Phase 1–3 contracts are broken.

**Resolved decisions carried forward:**

- Wake engine decision: **ship Porcupine path first** (free-tier keyword, `jarvis` keyword); OpenWakeWord is a parallel code path that can be toggled via `tauri.conf.json` / settings. Both paths share the same `WakeDetector` trait.
- Haiku model string: `claude-haiku-4-5` (as in SPEC). API key stored in OS keychain via `keytar` (Node side) or `keyring` crate (Rust side) — never in SQLite or env committed to git.
- App index: Windows registry scan (`HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall` + Start Menu `.lnk` crawl). macOS Spotlight / `/Applications` scan is deferred until macOS milestone.

---

## Architecture delta from Phase 3

```
Phase 3 entry point:       GlobalHotkey → Orch
Phase 4 additional path:   WakeDetector → Orch  (runs on its own thread, gated by is_paused)

New Rust modules:
  audio/wake/              trait WakeDetector + PorcupineBackend + OpenWakeWordBackend
  ai/                      mod ai_mode — calls Anthropic HTTP, returns structured ActionPlan
  apps/                    mod app_index — scan + cache installed apps, fuzzy resolve name→path

New IPC events:
  wake-detected            { backend: "porcupine"|"oww" }   → HUD transitions to listening
  ai-thinking              { node_id }                       → HUD shows thinking indicator
  ai-response              { text, actions: ActionPlan[] }   → HUD shows result
  app-index-ready          { count: usize }                  → settings UI, optional
```

**Key design rule (unchanged):** all new modules are pure logic. `lib.rs` orchestrates. `ai/` never imports `audio/`; `apps/` never imports `commands/`.

---

## Dependency graph

```
T4-1: WakeDetector trait + Porcupine backend
  |
  +---> T4-2: OpenWakeWord backend   (parallel, same trait; can ship after T4-1)
  |
  +---> T4-5: Wake path integration (lib.rs wires WakeDetector → same Orch pipeline as hotkey)
                |
                +---> T4-8: End-to-end wake + pipeline integration

T4-3: Haiku ai_mode — HTTP client + prompt + response parse
  |
  +---> T4-4: API key storage (OS keychain) + Settings IPC
  |
  +---> T4-6: ai_mode executor branch (lib.rs: if node.ai_mode → call ai/, else existing path)
                |
                +---> T4-8

T4-7: App index (scan + cache + fuzzy resolve)          [independent after Phase 3 T1 types]
  |
  +---> T4-8

T4-8: Integration + HUD updates (thinking indicator, wake-detected event)
  |
  +---> T4-9: Quality gates + docs
```

---

## Tasks

### Task T4-1: `WakeDetector` trait + Porcupine backend [M]

**Description:**

Add `jarvis/src-tauri/src/audio/wake/mod.rs` defining a `WakeDetector` trait:

```rust
pub trait WakeDetector: Send + 'static {
    /// Feed one chunk of PCM (16kHz, i16, mono). Returns true when wake phrase detected.
    fn process_frame(&mut self, pcm: &[i16]) -> Result<bool, WakeError>;
    fn backend_name(&self) -> &'static str;
}
```

Implement `PorcupineBackend` in `audio/wake/porcupine.rs` using the `pv_porcupine` crate (or raw FFI to the Porcupine shared library bundled under `src-tauri/resources/`). The backend reads the access key from the OS keychain (same store as Haiku key, different service name). Model file: `jarvis_windows.ppn` (bundled via `tauri.conf.json` `bundle.resources`). Frame length and sample rate from Porcupine's C API.

The `PorcupineBackend` is constructed at app start if `wake_engine = "porcupine"` in the persisted settings row (new `settings` table, single-row — see T4-4). If the key or model file is missing, backend falls back to hotkey-only mode and logs a warning (never panics or blocks startup).

**Acceptance criteria:**

- `WakeDetector` trait compiles with both backends behind `#[cfg]` gates (`feature = "porcupine"`, `feature = "oww"`).
- `PorcupineBackend::process_frame` returns `Ok(true)` in a unit test where a known-wake PCM fixture is fed in (or a mock struct that satisfies the trait).
- Missing key/model → app still starts in hotkey-only mode; warning logged.
- `cargo clippy -- -D warnings` clean on this module.

**Verification:**

- `cargo test audio::wake::` passes.
- Manual: speak wake word → observe `wake-detected` event in DevTools (after T4-5 wires it).

**Dependencies:** T1 (scaffold), Phase 3 T4a (mic capture already exists — reuse the PCM channel).

**Files:**

- `jarvis/src-tauri/src/audio/wake/mod.rs` (new)
- `jarvis/src-tauri/src/audio/wake/porcupine.rs` (new)
- `jarvis/src-tauri/Cargo.toml` (add `pv_porcupine` or raw lib dep)
- `jarvis/src-tauri/tauri.conf.json` (add `*.ppn`, Porcupine `.dll`/`.dylib` to `bundle.resources`)
- `jarvis/scripts/download-wake-models.ps1` (new — fetch `.ppn` + lib, never commit binaries)

---

### Task T4-2: OpenWakeWord backend [S–M]

**Description:**

Implement `OpenWakeWordBackend` in `audio/wake/oww.rs`. OpenWakeWord runs as a Python subprocess or via its C-API / ONNX runtime (prefer ONNX via `ort` crate to avoid Python dependency in the bundled app). The backend satisfies the same `WakeDetector` trait.

Use the community `hey_jarvis` or similar ONNX model (check license; document in README). Feed PCM frames from the same mic channel as Porcupine. Threshold configurable via the settings table (`oww_threshold f32`, default `0.5`).

This backend is gated behind `feature = "oww"` and is not enabled in the default Windows MVP build; it is the escape hatch when Porcupine licensing is blocked or a custom wake phrase is needed.

**Acceptance criteria:**

- `OpenWakeWordBackend` compiles under `--features oww`.
- `WakeDetector` trait satisfied (same test fixture approach as T4-1).
- Default build (`--no-default-features` or without `oww`) compiles without any OWW code.
- `backend_name()` returns `"oww"`.

**Verification:**

- `cargo test --features oww audio::wake::oww` passes.
- Feature-flag isolation: `cargo build` (no flags) does not include OWW symbols.

**Dependencies:** T4-1 (trait definition).

**Files:**

- `jarvis/src-tauri/src/audio/wake/oww.rs` (new)
- `jarvis/src-tauri/Cargo.toml` (add `ort` crate behind `oww` feature)
- `jarvis/scripts/download-oww-model.ps1` (new — fetch `.onnx`)

---

### Task T4-3: Haiku `ai_mode` — HTTP client + prompt contract [M]

**Description:**

Add `jarvis/src-tauri/src/ai/mod.rs`. This module is the **only** place in the codebase that calls the Anthropic API. Expose one async function:

```rust
pub async fn run_ai_mode(
    node: &CommandNode,
    transcript: &str,
    api_key: &str,
) -> Result<AiResponse, AiError>
```

Where `AiResponse` contains:
- `text: String` — the reply shown in HUD
- `actions: Vec<Action>` — zero or more actions to execute (same `Action` enum from Phase 1 T2)

The function calls `https://api.anthropic.com/v1/messages` with model `claude-haiku-4-5`, `max_tokens: 512`. System prompt (stored as `sub_prompt` on the `CommandNode`, falls back to a default): instructs the model to respond in structured JSON with `{ "text": "...", "actions": [...] }`. Parse response; on parse failure, return `AiResponse { text: raw_reply, actions: vec![] }` (graceful degradation).

Timeout: **10 seconds** hard ceiling; on timeout, surface an `AiError::Timeout` that the executor translates to an `action-status` event with a user-friendly message.

**Never** log or persist the API key or the response payload beyond the current call stack.

**Acceptance criteria:**

- `run_ai_mode` compiles with a mock HTTP client in tests (use `mockito` or `httpmock`).
- Successful mock response → `AiResponse` parsed correctly.
- Malformed JSON response → graceful degradation (no panic, `text` contains raw reply).
- Timeout (mock slow server) → `AiError::Timeout` returned.
- No API key appears in any log line (`cargo test -- --nocapture` inspected).

**Verification:**

- `cargo test ai::` with mock server.
- Manual (after T4-4 wires key): toggle `ai_mode: true` on seed node → speak → see HUD `ai-thinking` + response (after T4-6 wires it).

**Dependencies:** Phase 1 T2 (`Action` enum, `CommandNode`).

**Files:**

- `jarvis/src-tauri/src/ai/mod.rs` (new)
- `jarvis/src-tauri/Cargo.toml` (add `reqwest` with `rustls`, `tokio`, `mockito` dev-dep)

---

### Task T4-4: API key storage + Settings IPC + Settings UI [M]

**Description:**

**Rust side:** Add a `settings` table (single row, upsert pattern) to the existing SQLite DB:

```sql
CREATE TABLE IF NOT EXISTS settings (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  wake_engine TEXT NOT NULL DEFAULT 'hotkey',
  oww_threshold REAL NOT NULL DEFAULT 0.5,
  porcupine_access_key_stored INTEGER NOT NULL DEFAULT 0,  -- flag only; key in keychain
  anthropic_key_stored INTEGER NOT NULL DEFAULT 0,         -- flag only; key in keychain
  ai_mode_global INTEGER NOT NULL DEFAULT 0,
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

API keys stored via the `keyring` crate (`service = "jarvis-anthropic"`, `service = "jarvis-porcupine"`). Expose Tauri commands:

- `get_settings() → SettingsPayload`
- `save_api_key(service: String, key: String) → Result<()>` — writes to OS keychain
- `delete_api_key(service: String) → Result<()>`
- `update_settings(patch: SettingsPatch) → Result<()>`

**React side:** Add `src/components/Settings/SettingsPanel.tsx`. Accessible from tray "Settings" menu item (add to tray in T4-5). Shows:
- Wake engine selector (hotkey / porcupine / oww)
- API key input for Porcupine (masked, save/clear)
- API key input for Anthropic (masked, save/clear)
- Global ai_mode toggle
- "App Index" status (count from `app-index-ready` event, "Scanning…" / "N apps indexed")

**Acceptance criteria:**

- `save_api_key` writes to OS keychain; value not present in SQLite dump.
- `get_settings` returns flag `anthropic_key_stored: true` after key saved (does not return key value).
- `delete_api_key` removes from keychain and flips flag.
- UI: key input masked; Save/Clear buttons work; wake engine change persists across restart.
- `cargo test db::settings` passes.

**Verification:**

- `cargo test db::settings` + `cargo test commands::save_api_key`.
- Manual: open Settings from tray, save Anthropic key, restart app, flag shown as stored.

**Dependencies:** Phase 1 T2 (DB), Phase 1 T8 (tray — add Settings item).

**Files:**

- `jarvis/src-tauri/src/db/settings.rs` (new)
- `jarvis/src-tauri/src/db/mod.rs` (add settings module)
- `jarvis/src-tauri/Cargo.toml` (add `keyring`)
- `jarvis/src-tauri/src/lib.rs` (register new Tauri commands)
- `jarvis/src/components/Settings/SettingsPanel.tsx` (new)
- `jarvis/src/store/settingsStore.ts` (new Zustand store)
- `jarvis/src-tauri/src/tray.rs` (add "Settings" menu item)

---

### Checkpoint A — Wake + AI foundations

Before proceeding to T4-5/T4-6:

- [ ] `WakeDetector` trait + Porcupine backend unit tests green.
- [ ] OWW backend compiles behind feature flag; feature isolation confirmed.
- [ ] Haiku client unit tests green with mock server.
- [ ] OS keychain read/write works on Windows; key never surfaces in logs.
- [ ] Settings table persists across restart; UI shows correct stored-flag state.
- [ ] `cargo clippy -- -D warnings` clean across all new modules.

---

### Task T4-5: Wake path integration — `lib.rs` orchestrator [M]

**Description:**

Extend `lib.rs` to support a second trigger path alongside the hotkey:

1. At app start, read `settings.wake_engine`. If `porcupine` or `oww`, construct the appropriate `WakeDetector` impl and spawn a **dedicated wake thread** (`std::thread::spawn`).
2. The wake thread feeds PCM frames from a **secondary mic channel** (low-level, continuous — separate from the Whisper capture channel which is only active post-trigger). Use a ring buffer / shared channel; keep CPU budget low (Porcupine's frame processing is ~0.1ms/frame).
3. On `WakeDetector::process_frame` returning `Ok(true)`: send a message on an `mpsc` channel to the main async runtime → same `start_pipeline()` path as the hotkey handler. Emit `wake-detected { backend }` IPC event to React.
4. Gate: if `is_paused` flag is set, the wake thread discards frames and skips `start_pipeline`.
5. If wake engine is `hotkey`, no wake thread is started (existing behavior, fully backward compatible).

**Acceptance criteria:**

- Wake thread spawns only when `wake_engine ≠ "hotkey"`.
- Hotkey path unchanged; existing E2E checklist still passes.
- Speak wake word → HUD appears and enters `listening` phase (manual verification with Porcupine key).
- `is_paused = true` suppresses wake detection.
- No CPU spike when idle (< 2% additional on reference hardware, measured with Task Manager).
- `cargo test` still fully green.

**Verification:**

- Manual: configure Porcupine key in Settings → speak "Jarvis" → HUD appears.
- Manual: tray Pause → speak wake word → HUD does not appear.
- `cargo test` passes.

**Dependencies:** T4-1 or T4-2 (backend), T4-4 (settings read), Phase 1 T4a (mic capture).

**Files:**

- `jarvis/src-tauri/src/lib.rs` (wake thread spawn, event emit, pipeline gate)
- `jarvis/src-tauri/src/audio/mod.rs` (secondary PCM channel for wake, low-level)

---

### Task T4-6: `ai_mode` executor branch [S–M]

**Description:**

Extend `lib.rs` orchestrator's post-match step: after `Matcher` returns a `MatchResult`, check `node.ai_mode`:

- **`ai_mode: false` (default):** existing executor path (Phase 1 T7) — no change.
- **`ai_mode: true`:** 
  1. Retrieve Anthropic key from keychain. If missing → emit `action-status { text: "AI mode requires an Anthropic API key. Add it in Settings." }` → skip to `done`.
  2. Emit `ai-thinking { node_id }` IPC event → HUD enters thinking state.
  3. `await run_ai_mode(node, transcript, key)`.
  4. Emit `ai-response { text, actions }` IPC event.
  5. Execute returned `actions` through the existing executor (reuse T7 exactly — no duplication).
  6. Transition HUD to `executing` → `done`.

Add `ai_mode: bool` field to `CommandNode` struct and SQLite `command_nodes` table (migration: `ALTER TABLE command_nodes ADD COLUMN ai_mode INTEGER NOT NULL DEFAULT 0`). Bump schema version.

**Acceptance criteria:**

- `ai_mode: false` node: behavior identical to Phase 1–3 (no regression).
- `ai_mode: true` node, key present: `ai-thinking` event fires; `ai-response` event fires with parsed text; actions execute.
- `ai_mode: true` node, key missing: user-friendly `action-status` message, no panic.
- `cargo test commands::` still green.
- Schema migration runs without data loss (existing seed commands preserved).

**Verification:**

- `cargo test` — including DB migration idempotency test.
- Manual: seed one `ai_mode: true` node via Settings or direct DB edit → speak trigger → verify `ai-thinking` indicator on HUD, response text shown, action executes.

**Dependencies:** T4-3 (ai module), T4-4 (keychain read), Phase 1 T7 (executor reuse).

**Files:**

- `jarvis/src-tauri/src/ai/mod.rs` (already created in T4-3)
- `jarvis/src-tauri/src/db/models.rs` (add `ai_mode` field)
- `jarvis/src-tauri/src/lib.rs` (branch in post-match)
- `jarvis/src-tauri/src/db/mod.rs` (migration)
- `jarvis/src/store/hudStore.ts` (handle `ai-thinking` + `ai-response` events)
- `jarvis/src/components/HUD/HudOverlay.tsx` (thinking indicator, response text display)

---

### Task T4-7: App index — scan, cache, fuzzy resolve [M]

**Description:**

Add `jarvis/src-tauri/src/apps/mod.rs`. Responsible for building and querying a local index of installed applications so `OpenApp` can accept a friendly name (`"Discord"`, `"VS Code"`) rather than requiring a full path.

**Index build (Windows):**

1. Crawl `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall` and `HKCU\...\Uninstall` — extract `DisplayName` + `DisplayIcon` (or `InstallLocation`).
2. Crawl `%APPDATA%\Microsoft\Windows\Start Menu` and `%ProgramData%\Microsoft\Windows\Start Menu` for `.lnk` files — resolve targets via Windows Shell API (call via `windows-rs` crate).
3. Deduplicate by resolved path. Store in an in-memory `HashMap<String, PathBuf>` (app name → exe path).
4. Persist cache to a `app_index` table in SQLite (columns: `name TEXT`, `exe_path TEXT`, `source TEXT`, `updated_at TEXT`) for fast startup (no full rescan needed every launch — rescan if cache is > 24h old or explicitly triggered).

**Resolve function:**

```rust
pub fn resolve_app(name: &str, index: &AppIndex) -> Option<PathBuf>
```

Uses `rapidfuzz` (already in scope from Phase 2) with threshold `0.75`. Returns best match or `None`.

**Executor integration:** In `executor.rs`, `Action::OpenApp { name, path }` — if `path` is empty/None, call `resolve_app(name, index)`. If found, use resolved path; otherwise fall back to `cmd /C start <name>` (existing behavior, keep as last-resort).

**Acceptance criteria:**

- `AppIndex::build_windows()` returns > 0 entries on a standard Windows 11 dev machine.
- `resolve_app("notepad", &index)` returns the Notepad path without a full path hint.
- Cache written to SQLite; on cold start, read from cache (no registry scan if < 24h old).
- `cargo test apps::` passes (mock index for unit tests; integration test with real registry behind `#[cfg(target_os = "windows")]`).
- `app-index-ready { count }` IPC event emitted after build completes.

**Verification:**

- `cargo test apps::` passes.
- Manual: on Windows, `resolve_app("discord")` resolves correctly in test binary.
- Settings panel shows indexed app count.

**Dependencies:** Phase 1 T2 (SQLite), Phase 2 `rapidfuzz` integration.

**Files:**

- `jarvis/src-tauri/src/apps/mod.rs` (new)
- `jarvis/src-tauri/src/apps/scanner_windows.rs` (new)
- `jarvis/src-tauri/src/commands/executor.rs` (wire resolve_app)
- `jarvis/src-tauri/src/db/mod.rs` (add app_index table)
- `jarvis/src-tauri/Cargo.toml` (add `windows-rs` features for shell / registry)

---

### Checkpoint B — Integration-ready

Before T4-8:

- [ ] Wake thread running; `wake-detected` event visible in DevTools.
- [ ] `ai_mode` branch fires `ai-thinking` + `ai-response` with live Anthropic key.
- [ ] App index built on startup; `app-index-ready` count > 0 on Windows dev machine.
- [ ] Schema migration idempotent on existing Phase 3 DB.
- [ ] All unit tests green. `cargo clippy -- -D warnings` clean.

---

### Task T4-8: End-to-end Phase 4 integration [M]

**Description:**

Wire remaining loose ends and verify all three Phase 4 features work in a single running app instance:

1. **HUD updates for Phase 4 events:**
   - `wake-detected`: HUD appears from idle (no hotkey needed), enters `listening` phase, shows backend name as subtle badge.
   - `ai-thinking`: New HUD sub-phase — show animated ellipsis / spinner inside the transcript area; text "Thinking…".
   - `ai-response`: Transition to showing AI reply text in transcript area before `executing`.
2. **Tray updates:** "Settings" opens `SettingsPanel`. Wake engine shown in tray tooltip (e.g. "JARVIS — Porcupine active").
3. **Startup sequencing in `lib.rs`:**
   - Init DB (including migration).
   - Load settings.
   - Build app index (async, non-blocking — emit `app-index-ready` when done).
   - Init wake engine (or skip if `hotkey`).
   - Register global hotkey (always, as fallback).
4. **Graceful degradation matrix** (all tested manually):

   | Wake engine | Anthropic key | App index | Expected behavior |
   |-------------|--------------|-----------|-------------------|
   | hotkey | absent | fresh | Phase 1–3 behavior unchanged |
   | porcupine | absent | cached | Wake works; `ai_mode` nodes show key-missing message |
   | porcupine | present | cached | Full Phase 4 path |
   | hotkey | present | fresh | Hotkey trigger + ai_mode works |

**Acceptance criteria:**

- Speak wake word → HUD → speak command → execute: full path end-to-end.
- `ai_mode: true` node: thinking indicator, AI text, action executes.
- `OpenApp("Discord")` with empty path field resolves via app index.
- All Phase 1–3 E2E checklist items still pass (no regression).
- `cargo test` and `npm run build` clean.

**Verification:** Manual E2E checklist in `todo4.md`.

**Dependencies:** T4-5, T4-6, T4-7.

**Files:**

- `jarvis/src-tauri/src/lib.rs` (startup sequence, event wiring)
- `jarvis/src/components/HUD/HudOverlay.tsx` (ai-thinking phase, badge)
- `jarvis/src/store/hudStore.ts` (new events)
- `jarvis/src-tauri/src/tray.rs` (tooltip, Settings item)

---

### Task T4-9: Quality gates + docs [S]

**Description:**

- Update `jarvis/README.md`: document Porcupine setup (access key, `.ppn` download script), OWW opt-in build flag, Anthropic BYOK instructions, app index behavior.
- Add `jarvis/scripts/download-wake-models.ps1` (Porcupine `.ppn` + shared lib) to prereqs section.
- `cargo fmt --check` clean across all new modules.
- `cargo clippy -- -D warnings` clean.
- `npm run lint` clean.
- `npm run test` (Vitest) — add at minimum one test for `settingsStore` and `hudStore` Phase 4 event handling.
- `npm run tauri build` produces `.exe` with Phase 4 features.
- Confirm `*.ppn`, `*.onnx`, `*.bin` model files excluded from git (`.gitignore` check).

**Acceptance criteria:**

- All quality gates listed above pass.
- README has a "Phase 4 features" section with step-by-step setup.
- Clean-clone → download models → build → install → E2E checklist passes.

**Dependencies:** T4-8.

**Files:**

- `jarvis/README.md`
- `jarvis/src/**/*.test.ts` (new Vitest tests for Phase 4 store logic)
- Minor cleanup anywhere flagged by clippy / eslint.

---

### Checkpoint C — Phase 4 complete

- [ ] All three Phase 4 features (wake word, `ai_mode`, app index) pass manual E2E checklist.
- [ ] No regression on Phase 1–3 checklist items.
- [ ] Quality gates clean: `cargo fmt`, `cargo clippy`, `npm run lint`, `npm run test`, `npm run tauri build`.
- [ ] README updated with Phase 4 prereqs and setup.
- [ ] Porcupine vs OWW decision documented in release notes (which ships by default, how to switch).
- [ ] Human sign-off before Phase 5 (code signing, auto-updater, macOS DMG).

---

## Risks and mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| Porcupine free tier keyword limited to `"jarvis"` (or requires paid for custom) | High if custom wake phrase needed | OWW backend is the escape hatch; both implemented behind trait |
| `windows-rs` shell/registry API surface complexity for `.lnk` resolution | Medium — scanner may miss apps | Implement multi-source fallback: registry + Start Menu + `%PATH%` scan; document known gaps |
| Anthropic API latency in `ai_mode` (network dependency) | Medium — HUD hangs | 10s hard timeout + `ai-thinking` indicator; users understand cloud RTT |
| `keyring` crate Windows Credential Manager behavior in sandboxed builds | Medium — Tauri bundle may restrict credential access | Test in bundled `.exe` early (not just `tauri dev`); document fallback (env var opt-in for CI) |
| OS keychain unavailable on CI runners (no GUI session) | Low for prod; blocks CI tests | Mock keychain in test builds; `#[cfg(test)]` shim |
| Schema migration on existing user DBs | Low risk — additive only | Use `ALTER TABLE … ADD COLUMN … DEFAULT` (SQLite safe); test migration idempotency |

---

## Later phases (out of scope for this plan)

- **Phase 5:** Code signing, Tauri auto-updater, Windows NSIS installer, macOS DMG + Notarization.
- **Phase 6:** Cloud command sync (opt-in), multi-device.
- **Phase 7:** Plugin system, third-party action types.

---

## References

- `SPEC.md` — Open Questions 1 (wake engine), 3 (monetization / BYOK), 5 (plugin system).
- `plan.md` (Phase 1) — architecture baseline, IPC contract, module boundaries.
- `BrainStorm.md` — HUD state machine (Phase 4 adds `ai-thinking` sub-state).