# JARVIS Phase 4 — Task Checklist (revised)

> Phases 1–3 complete. **Phase 4 scope:** wake word, **transcription backend choice** (local / OS / online API), app auto-detection. **No** Haiku / LLM `ai_mode` — only “AI” is **speech-to-text** models/providers.  
> Work each task to **Done** before the next unless the dependency graph allows parallel tracks.

---

## Checkpoint A prerequisites (before T4-5 / full orchestration)

- [x] **T4-1** `WakeDetector` trait + Porcupine backend *(implemented; see checklist below)*
- [x] **T4-2** OpenWakeWord backend (feature-gated) *(implemented)*
- [x] **T4-3** Transcription backend abstraction + local path + settings contract *(replaces old “Haiku client” milestone — STT only)*
- [x] **T4-4** Settings IPC + UI — wake, Porcupine, **STT provider**; **remove** Anthropic / global AI mode UI *(coordinate with T4-6)*

### Legacy note (codebase today)

Earlier Phase 4 work added **Haiku `ai_mode`** (`src/ai/`, Anthropic settings). That direction is **retired**. **T4-6** removes those artifacts after STT settings (T4-3/4) land.

---

## T4-1 · `WakeDetector` trait + Porcupine backend

- [x] `audio/wake/mod.rs` — `WakeDetector` trait with `process_frame` + `backend_name`
- [x] `audio/wake/porcupine.rs` — `PorcupineBackend` implements trait
- [x] Porcupine access key read from OS keychain at construction
- [x] `.ppn` model file + Porcupine shared lib in `bundle.resources` (`prebuild` / `fetch-wake-models.mjs`)
- [x] `scripts/download-wake-models.ps1` fetches binaries (not committed)
- [x] `.gitignore` covers `*.ppn`, `*.dll` model files
- [x] Missing key or model → app starts in hotkey-only mode (no panic, warning logged)
- [x] `cargo test audio::wake::` passes
- [x] `cargo clippy` clean on new wake sources (no new warnings in `audio/wake`)

---

## T4-2 · OpenWakeWord backend

- [x] `audio/wake/oww.rs` — `OpenWakeWordBackend` implements `WakeDetector` trait
- [x] ONNX runtime via `ort` crate; no Python dependency in bundled app
- [x] `oww_threshold` persisted in settings + passed into `try_new` (persisted in settings; wake thread wiring **T4-5**)
- [x] Gated behind `feature = "oww"` in `Cargo.toml`
- [x] Default build compiles without OWW symbols (`cargo build` — no `oww` feature)
- [x] `scripts/download-oww-model.ps1` fetches `.onnx`
- [x] `cargo test --features oww audio::wake::oww` passes
- [x] `backend_name()` returns `"oww"`

---

## T4-3 · Transcription engine selection (local / OS / online API)

**Replaces obsolete milestone “Haiku HTTP client” — product has no command LLM.**

- [x] `TranscriptionBackend` (or equivalent) trait / enum: **local on-device**, **OS speech API**, **remote HTTP STT**
- [x] Settings row / keys for `stt_provider` + remote endpoint metadata (secrets via keychain, not SQLite)
- [x] **Local** path: current Whisper (or bundled) pipeline selected by default; behavior matches Phase 1–3 when chosen
- [x] **OS** path: Windows implementation or documented stub + `cfg` for other OSes
- [x] **Remote** path: HTTP contract documented; timeout; errors surfaced to HUD/status (no key in logs)
- [x] `cargo test` for transcription module(s)
- [x] `cargo clippy -- -D warnings` on touched modules

---

## T4-4 · Settings + IPC + Settings UI (revised)

### Rust / DB

- [x] Settings include wake engine, Porcupine flag, **STT provider** fields; remove **`global_ai_mode`** / **`anthropic_key_stored`** when T4-6 lands (or single migration with T4-6) *(T4-6 complete; legacy keys removed)*
- [x] Keychain: Porcupine + **remote STT** secrets only as needed — **no** Anthropic service
- [x] `get_settings()` / `update_settings()` reflect new fields; migrations idempotent

### React

- [x] Wake engine selector (hotkey / porcupine / oww) persists
- [x] **Transcription provider** UI: local / OS / online (labels clear)
- [x] Remote STT: masked key + endpoint fields as required by T4-3 contract
- [x] **Remove** global `ai_mode` toggle and Anthropic key panel *(T4-6)*
- [x] Porcupine access key: masked, Save / Clear
- [x] "App Index" status from `app-index-ready`
- [x] Tray "Settings" opens panel

---

## ~~T4-3 (obsolete) · Haiku `ai_mode` HTTP client~~ — **DO NOT EXTEND**

*Retired. Removal tracked in **T4-6**.*

---

## ~~T4-4 (obsolete section) · Anthropic API for commands~~ — **DO NOT EXTEND**

*Retired. Replaced by T4-3/T4-4 STT + T4-6 cleanup.*

---

## ✅ Checkpoint A

- [x] `WakeDetector` trait + Porcupine unit tests green
- [x] OWW compiles behind `oww` feature
- [x] Transcription abstraction + local path wired; settings keys defined
- [x] OS keychain read/write works in bundled `.exe` on Windows (Porcupine / STT secrets) *(release smoke test recommended; dev uses same `keyring` path)*
- [x] Key material never surfaces in logs *(see `keychain` tests + no secret logging in module)*
- [x] `cargo clippy --lib -- -D warnings` clean — default build and `--features oww` *(test targets still have pre-existing lints in `lib.rs` / `executor.rs` if you run `--all-targets`)*

---

## T4-5 · Wake path integration (`lib.rs` orchestrator)

- [ ] On startup, read `settings.wake_engine`
- [ ] If `porcupine` or `oww`: spawn dedicated wake thread
- [ ] Wake thread feeds PCM from secondary mic channel (ring buffer / channel)
- [ ] `Ok(true)` from `process_frame` → main runtime → `start_pipeline()`
- [ ] `wake-detected { backend }` IPC to React
- [ ] `is_paused = true` → wake thread discards frames, skips pipeline
- [ ] `wake_engine = "hotkey"` → no wake thread (backward compatible)
- [ ] Hotkey path unchanged — Phase 1–3 E2E still pass
- [ ] CPU overhead < 2% idle (Task Manager spot check)
- [ ] `cargo test` fully green

---

## ~~T4-6 (obsolete) · `ai_mode` executor branch~~ — **REPLACED BY T4-6 CLEANUP**

*No LLM post-match. Matcher uses transcript string only.*

---

## T4-6 · Legacy AI removal — Haiku, `ai_mode`, `ai` module

- [x] Delete or gut `src/ai/` (Anthropic / OpenAI-compatible LLM client used for **command** reasoning)
- [x] Remove `ai_mode` from `CommandNode`, DB, migrations; update seeds / editor / IPC validation
- [x] Remove `global_ai_mode`, `anthropic_key_stored` from settings + keychain helpers for Anthropic
- [x] Remove executor `run_ai_mode` / preview HTTP calls to Anthropic; remove related tests
- [x] Remove HUD / store handling for `ai-thinking` / `ai-response` if present *(none in codebase)*
- [x] Settings UI: no Anthropic copy; no “AI mode” for commands
- [x] `npm run test` + `cargo test` green; `rg` clean for `claude-haiku`, `run_ai_mode` (except changelog)

---

## T4-7 · App index — scan, cache, fuzzy resolve

- [ ] `apps/mod.rs` + `apps/scanner_windows.rs`
- [ ] Registry scan: `HKLM` + `HKCU` Uninstall keys → name + exe path
- [ ] Start Menu `.lnk` crawl via `windows-rs`
- [ ] Deduplicate by resolved path; in-memory map
- [ ] `app_index` SQLite table; cold start < 24h uses cache
- [ ] Stale cache → background rebuild → `app-index-ready`
- [ ] `resolve_app(name, index)` — `rapidfuzz` threshold `0.75`
- [ ] `executor.rs`: `OpenApp` empty path → `resolve_app` → `cmd /C start` fallback
- [ ] `cargo test apps::` passes

---

## ✅ Checkpoint B

- [ ] Wake thread; `wake-detected` in DevTools
- [ ] STT provider saved and applied (local verified)
- [ ] T4-6 complete before release candidate
- [ ] App index built; count > 0 on Windows dev box
- [ ] Schema migrations idempotent
- [ ] All unit tests green; `cargo clippy -- -D warnings` clean

---

## T4-8 · End-to-end Phase 4 integration

### HUD

- [ ] `wake-detected` → HUD from idle, `listening`, backend badge
- [ ] **No** LLM “Thinking…” state (transcript only)

### Tray

- [x] "Settings" opens `SettingsPanel`
- [ ] Tooltip reflects wake engine (e.g. "JARVIS — Porcupine active")

### Startup (`lib.rs`)

- [ ] DB init → settings → app index (async) → wake init → hotkey
- [ ] Failures non-fatal with clear warnings

### Degradation matrix (manual)

- [ ] hotkey + local STT + index → baseline Phase 1–3 ✓
- [ ] porcupine + local STT + cached index → wake + commands ✓
- [ ] remote STT misconfigured → clear error, no panic ✓

### Regression

- [ ] Phase 1–3 E2E checklist passes

### Build

- [ ] `cargo test` green
- [ ] `npm run build` green

---

## T4-9 · Quality gates + docs

- [ ] `cargo fmt --check` clean
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `npm run lint` clean
- [ ] Vitest: `settingsStore` — STT / wake events (not LLM)
- [ ] `npm run tauri build` produces `.exe`
- [ ] `.gitignore`: `*.ppn`, `*.onnx`, `*.bin` excluded
- [ ] README "Phase 4 features": Porcupine, OWW flag, **STT providers**, app index — **not** Anthropic command AI
- [ ] `scripts/download-wake-models.ps1` in README prereqs

---

## ✅ Checkpoint C — Phase 4 complete

- [ ] Wake word E2E: speak → HUD → command executes ✓
- [ ] Transcription provider choice works (local + at least one alternate path or documented roadmap) ✓
- [ ] `OpenApp("Discord")` without path resolves via index ✓
- [ ] No regression on Phase 1–3 ✓
- [ ] Quality gates pass ✓
- [ ] README updated ✓
- [ ] **Human sign-off** — Phase 5 (signing, updater, macOS DMG) ✓
