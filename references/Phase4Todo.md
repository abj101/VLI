# JARVIS Phase 4 — Task Checklist

> Phases 1–3 complete. This checklist covers Phase 4: Wake Word, `ai_mode`, App Auto-Detection.
> Work each task to **Done** before moving to the next unless the dependency graph allows parallel tracks.

---

## Checkpoint A prerequisites (complete before T4-5/T4-6)

- [x] **T4-1** `WakeDetector` trait + Porcupine backend
- [ ] **T4-2** OpenWakeWord backend (feature-gated)
- [ ] **T4-3** Haiku `ai_mode` HTTP client
- [ ] **T4-4** API key storage + Settings IPC + Settings UI

---

## T4-1 · `WakeDetector` trait + Porcupine backend

- [x] `audio/wake/mod.rs` created — `WakeDetector` trait with `process_frame` + `backend_name`
- [x] `audio/wake/porcupine.rs` — `PorcupineBackend` implements trait
- [x] Porcupine access key read from OS keychain at construction
- [x] `.ppn` model file + Porcupine shared lib added to `bundle.resources` (`prebuild` runs `fetch-wake-models.mjs` so files exist before bundle)
- [x] `scripts/download-wake-models.ps1` fetches binaries (not committed to git)
- [x] `.gitignore` covers `*.ppn`, `*.dll` model files
- [x] Missing key or model → app starts in hotkey-only mode (no panic, warning logged)
- [x] `cargo test audio::wake::` passes
- [x] `cargo clippy` clean on new wake sources (no new warnings in `audio/wake`)

---

## T4-2 · OpenWakeWord backend

- [ ] `audio/wake/oww.rs` — `OpenWakeWordBackend` implements `WakeDetector` trait
- [ ] ONNX runtime via `ort` crate; no Python dependency in bundled app
- [ ] `oww_threshold` read from settings table (default `0.5`)
- [ ] Gated behind `feature = "oww"` in `Cargo.toml`
- [ ] Default build compiles without OWW symbols (`cargo build` — no `oww` feature)
- [ ] `scripts/download-oww-model.ps1` fetches `.onnx`
- [ ] `cargo test --features oww audio::wake::oww` passes
- [ ] `backend_name()` returns `"oww"`

---

## T4-3 · Haiku `ai_mode` HTTP client

- [ ] `src/ai/mod.rs` created — `run_ai_mode(node, transcript, api_key)` async fn
- [ ] `reqwest` with `rustls` added to `Cargo.toml`
- [ ] Calls `https://api.anthropic.com/v1/messages` with `claude-haiku-4-5`
- [ ] `sub_prompt` on `CommandNode` used as system prompt (fallback to default)
- [ ] Response parsed into `AiResponse { text, actions: Vec<Action> }`
- [ ] Malformed JSON → graceful degradation (raw text, no panic)
- [ ] 10-second hard timeout → `AiError::Timeout` returned
- [ ] API key does not appear in any log line (`--nocapture` inspected)
- [ ] `mockito` (or `httpmock`) used in tests — no real network calls in CI
- [ ] `cargo test ai::` passes

---

## T4-4 · API key storage + Settings IPC + Settings UI

### Rust / DB
- [ ] `settings` table created (single-row upsert pattern; schema in `plan4.md`)
- [ ] `keyring` crate added to `Cargo.toml`
- [ ] `save_api_key(service, key)` writes to OS keychain — not SQLite
- [ ] `delete_api_key(service)` removes from keychain, flips stored flag
- [ ] `get_settings()` returns flag only (`anthropic_key_stored: bool`) — never key value
- [ ] `update_settings(patch)` persists wake engine + threshold changes
- [ ] `cargo test db::settings` passes
- [ ] Schema migration idempotent on existing Phase 3 DB

### React
- [ ] `SettingsPanel.tsx` created in `src/components/Settings/`
- [ ] `settingsStore.ts` Zustand store created
- [ ] Wake engine selector (hotkey / porcupine / oww) persists across restart
- [ ] Anthropic API key input: masked, Save / Clear buttons functional
- [ ] Porcupine access key input: masked, Save / Clear buttons functional
- [ ] Global `ai_mode` toggle visible and persists
- [ ] "App Index" status shows count from `app-index-ready` event
- [ ] Tray "Settings" menu item opens `SettingsPanel`

---

## ✅ Checkpoint A

- [ ] `WakeDetector` trait + Porcupine unit tests green
- [ ] OWW compiles behind `oww` feature; feature isolation confirmed
- [ ] Haiku client unit tests green (mock server only)
- [ ] OS keychain read/write works in bundled `.exe` on Windows
- [ ] Key never surfaces in logs
- [ ] Settings table persists across restart; UI shows correct flag state
- [ ] `cargo clippy -- -D warnings` clean across all new modules

---

## T4-5 · Wake path integration (`lib.rs` orchestrator)

- [ ] On startup, read `settings.wake_engine`
- [ ] If `porcupine` or `oww`: spawn dedicated wake thread
- [ ] Wake thread feeds PCM frames from secondary mic channel (ring buffer / channel)
- [ ] `Ok(true)` from `process_frame` → send message to main async runtime → `start_pipeline()`
- [ ] `wake-detected { backend }` IPC event emitted to React
- [ ] `is_paused = true` → wake thread discards frames, skips pipeline
- [ ] If `wake_engine = "hotkey"`: no wake thread spawned (backward compatible)
- [ ] Hotkey path unchanged — all Phase 1–3 E2E items still pass
- [ ] CPU overhead < 2% additional when idle (verified via Task Manager)
- [ ] `cargo test` fully green

---

## T4-6 · `ai_mode` executor branch

### DB / model
- [ ] `ai_mode: bool` field added to `CommandNode` struct
- [ ] `ALTER TABLE command_nodes ADD COLUMN ai_mode INTEGER NOT NULL DEFAULT 0` migration
- [ ] Schema version bumped; migration runs automatically on startup
- [ ] Existing seed commands preserved after migration

### Orchestrator
- [ ] Post-match: check `node.ai_mode`
- [ ] `ai_mode: false` → existing executor path (no change)
- [ ] `ai_mode: true`, key present → `ai-thinking` event → `run_ai_mode()` → `ai-response` event → executor runs returned actions
- [ ] `ai_mode: true`, key absent → user-friendly `action-status` message, no panic, HUD → `done`
- [ ] `ai-thinking` IPC event handled in `hudStore.ts`
- [ ] `ai-response` IPC event handled in `hudStore.ts`

### HUD
- [ ] Thinking sub-phase shows animated ellipsis / spinner in transcript area
- [ ] AI response text shown before transitioning to `executing`
- [ ] `cargo test commands::` still green (no regression)

---

## T4-7 · App index — scan, cache, fuzzy resolve

### Scanner
- [ ] `apps/mod.rs` and `apps/scanner_windows.rs` created
- [ ] Registry scan: `HKLM` + `HKCU` Uninstall keys, extracts name + exe path
- [ ] Start Menu `.lnk` crawl (`%APPDATA%` + `%ProgramData%`) via `windows-rs` shell API
- [ ] Deduplication by resolved path
- [ ] In-memory `HashMap<String, PathBuf>` built after scan

### Cache
- [ ] `app_index` table in SQLite (name, exe_path, source, updated_at)
- [ ] Cold start: read from cache if < 24h old (skip registry scan)
- [ ] Stale cache (> 24h) → rebuild in background, then update cache
- [ ] `app-index-ready { count }` IPC event emitted after build

### Resolve
- [ ] `resolve_app(name, index)` uses `rapidfuzz` threshold `0.75`
- [ ] Returns `Some(PathBuf)` on match, `None` otherwise
- [ ] `executor.rs` updated: `OpenApp` with empty path → `resolve_app` → fallback to `cmd /C start`
- [ ] `cargo test apps::` passes (mock index for unit tests)
- [ ] Windows integration test (real registry) behind `#[cfg(target_os = "windows")]`

---

## ✅ Checkpoint B

- [ ] Wake thread running; `wake-detected` event visible in DevTools
- [ ] `ai_mode` branch: `ai-thinking` + `ai-response` fire with live Anthropic key
- [ ] App index built on startup; count > 0 on Windows dev machine shown in Settings
- [ ] Schema migration idempotent on existing Phase 3 DB
- [ ] All unit tests green
- [ ] `cargo clippy -- -D warnings` clean

---

## T4-8 · End-to-end Phase 4 integration

### HUD updates
- [ ] `wake-detected` → HUD appears from idle, enters `listening`, shows backend badge
- [ ] `ai-thinking` → spinner / "Thinking…" visible in transcript area
- [ ] `ai-response` → AI text displayed before `executing` phase

### Tray updates
- [ ] "Settings" tray item opens `SettingsPanel`
- [ ] Tray tooltip reflects active wake engine (e.g. "JARVIS — Porcupine active")

### Startup sequencing (`lib.rs`)
- [ ] Order: DB init (migration) → load settings → app index build (async, non-blocking) → wake engine init → hotkey register
- [ ] Each step logs outcome; failures are non-fatal with clear warning

### Degradation matrix — manual verification
- [ ] hotkey + no key + fresh index → Phase 1–3 behavior unchanged ✓
- [ ] porcupine + no key + cached index → wake works; `ai_mode` shows key-missing message ✓
- [ ] porcupine + key present + cached index → full Phase 4 path ✓
- [ ] hotkey + key present + fresh index → hotkey trigger + `ai_mode` works ✓

### Regression
- [ ] Phase 1–3 E2E checklist: hotkey → "open notepad" → Notepad opens ✓
- [ ] Phase 1–3 E2E checklist: hotkey → "open github" → browser opens ✓
- [ ] Phase 1–3 E2E checklist: nonsense for 5s → graceful dismiss ✓
- [ ] Phase 1–3 E2E checklist: stop button cancels pipeline ✓
- [ ] Phase 1–3 E2E checklist: tray pause suppresses hotkey ✓

### Build
- [ ] `cargo test` green
- [ ] `npm run build` green

---

## T4-9 · Quality gates + docs

- [ ] `cargo fmt --check` clean across all Phase 4 modules
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `npm run lint` clean
- [ ] `npm run test` (Vitest) — `settingsStore` Phase 4 event test added
- [ ] `npm run test` (Vitest) — `hudStore` `ai-thinking` / `ai-response` event test added
- [ ] `npm run tauri build` produces `.exe` with Phase 4 features
- [ ] `.gitignore` confirmed: `*.ppn`, `*.onnx`, `*.bin` excluded
- [ ] `README.md` "Phase 4 features" section added:
  - [ ] Porcupine setup (access key, `.ppn` download script)
  - [ ] OWW opt-in build flag (`--features oww`)
  - [ ] Anthropic BYOK instructions
  - [ ] App index behavior and known gaps documented
- [ ] `scripts/download-wake-models.ps1` listed in README prereqs

---

## ✅ Checkpoint C — Phase 4 complete

- [ ] Wake word (Porcupine): speak → HUD appears → command executes — end-to-end ✓
- [ ] `ai_mode`: speak trigger → thinking indicator → AI response → action executes ✓
- [ ] `OpenApp("Discord")` with no path hint → resolved via app index ✓
- [ ] No regression on Phase 1–3 E2E checklist ✓
- [ ] Quality gates: `cargo fmt`, `cargo clippy`, `npm run lint`, `npm run test`, `npm run tauri build` all pass ✓
- [ ] README Phase 4 section complete ✓
- [ ] Wake engine decision documented in release notes ✓
- [ ] **Human sign-off** — proceed to Phase 5 (code signing, auto-updater, macOS DMG) ✓