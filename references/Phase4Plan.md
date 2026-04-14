# Implementation Plan: JARVIS Phase 4 (revised)

## Scope

Phase 4 adds three capabilities that were explicitly deferred in Phase 1–3:

1. **Wake-word engine** — Porcupine (primary) or OpenWakeWord (fallback / custom phrase) replaces the hotkey-only trigger for always-on listening.
2. **Transcription engine selection** — The **only** “AI” in the product sense is **speech-to-text**: users choose a **transcription backend** — **local on-device** (bundled Whisper.cpp / similar), **OS-provided** (platform speech APIs, e.g. Windows speech recognition), or **remote HTTP API** (user-supplied endpoint + credentials). No separate LLM layer for command interpretation; matching stays fuzzy/exact as in Phase 1–3.
3. **App auto-detection** — `OpenApp` actions resolve against a live index of installed applications (Windows registry / Start Menu scan) instead of requiring the user to supply a raw executable path.

Phases 1–3 are complete. This plan **does not** add Anthropic Haiku, `ai_mode` on command nodes, or any secondary “assistant” LLM. Legacy code paths for those features are **removed** under Task T4-6 (see below).

**Resolved decisions carried forward:**

- Wake engine: **ship Porcupine path first** (free-tier keyword); OpenWakeWord toggled via settings / feature flag. Both share `WakeDetector` trait.
- Transcription: **default = local bundled model** for privacy; OS and remote are opt-in and clearly labeled in Settings.
- Remote STT: treat as **user BYOK** to a vendor endpoint (OpenAI-compatible or documented HTTP contract); keys in OS keychain, never in SQLite.
- App index: Windows registry + Start Menu scan first; macOS deferred.

---

## Architecture delta from Phase 3

```
Phase 3 entry point:       GlobalHotkey → Orch
Phase 4 additional path:   WakeDetector → Orch  (dedicated thread, gated by is_paused)

Rust modules:
  audio/wake/              trait WakeDetector + PorcupineBackend + OpenWakeWordBackend
  audio/transcription/     (or equivalent) trait TranscriptionBackend + LocalWhisper + OsStt + RemoteApi
  apps/                    app index — scan + cache, fuzzy resolve name→path

New IPC events:
  wake-detected            { backend: "porcupine"|"oww" }   → HUD → listening
  app-index-ready          { count: usize }                  → settings UI
  (transcript path unchanged: existing transcript-update / pipeline events — provider swap is internal)
```

**Key design rule (unchanged):** pure logic in modules; `lib.rs` orchestrates. Transcription backend does not import `commands/` matcher logic; matcher consumes final transcript string as today.

**Explicitly out of scope for Phase 4:** Haiku / `ai_mode` / `ai-thinking` / `ai-response` / `src/ai/` HTTP client for command reasoning — removed in T4-6.

---

## Dependency graph

```
T4-1: WakeDetector trait + Porcupine backend
  |
  +---> T4-2: OpenWakeWord backend   (parallel; same trait)
  |
  +---> T4-5: Wake path integration (lib.rs wires WakeDetector → same Orch pipeline as hotkey)
                |
                +---> T4-8: End-to-end wake + pipeline integration

T4-3: Transcription backend abstraction + settings (local + contract for OS + remote)
  |
  +---> T4-4: Settings IPC + UI (wake, Porcupine key, STT provider, remote key/endpoint flags)
  |
  +---> T4-6: Legacy AI removal (Haiku, ai_mode, ai module, global_ai_mode UI) — after T4-3/T4-4
                so STT story replaces “extra AI” in UX copy

T4-7: App index (scan + cache + fuzzy resolve)          [independent after Phase 3 types]
  |
  +---> T4-8

T4-8: Integration + HUD (wake badge, no LLM “thinking” phase)
  |
  +---> T4-9: Quality gates + docs
```

---

## Tasks

### Task T4-1: `WakeDetector` trait + Porcupine backend [M]

**Description:** Same as prior Phase 4 plan: trait + `PorcupineBackend`, key from keychain, bundled `.ppn`, graceful fallback to hotkey-only.

**Acceptance criteria:** Unchanged from original T4-1 (trait, tests, missing key/model → warning, no panic).

**Dependencies:** Phase 3 mic capture.

**Files:** `audio/wake/*`, `Cargo.toml`, `tauri.conf.json`, download scripts.

---

### Task T4-2: OpenWakeWord backend [S–M]

**Description:** Same as prior plan: `OpenWakeWordBackend` behind `feature = "oww"`, ONNX via `ort`.

**Acceptance criteria:** Unchanged (feature isolation, `backend_name()` = `"oww"`).

**Dependencies:** T4-1.

---

### Task T4-3: Transcription backend selection — abstraction + local path [M]

**Description:**

Introduce a **transcription** abstraction used by the listen pipeline (replacing a single hardcoded Whisper entry point as the only option):

- **Local on-device:** bundled Whisper.cpp (or current stack) — default.
- **OS-native:** platform speech recognition API (implementation may be Windows-first; stub or feature-gate others).
- **Remote API:** HTTP STT endpoint with user config (URL, model id if needed, API key in keychain).

Persist `stt_provider` (and related fields: e.g. remote endpoint id, flags) in the existing settings store. No LLM calls; only audio → text.

**Acceptance criteria:**

- User-facing enum or string contract for three provider classes documented in code + README.
- Local path preserves current behavior when selected (tests still pass).
- OS / remote may ship as stubs returning clear `NotImplemented` or behind `cfg` until implemented — but **settings + types** exist so UI can save choices.

**Verification:** `cargo test` for transcription module; manual: switch provider in Settings (once UI lands in T4-4) and verify pipeline uses local when selected.

**Dependencies:** Phase 3 STT pipeline hooks identified in `lib.rs` / `audio/`.

**Files (likely):** `audio/transcription/*.rs` (or `audio/stt/*.rs`), `db/settings.rs`, `lib.rs` orchestration.

---

### Task T4-4: Settings + IPC + Settings UI (revised) [M]

**Description:**

**Rust:** Extend settings for wake engine, Porcupine flags, **STT provider** and remote STT key/endpoint flags (keychain for secrets). **Remove** persistence and commands tied to Anthropic / `global_ai_mode` / `ai_mode` — those belong to T4-6 if still present in code.

**React:** Settings panel: wake engine, Porcupine key, **transcription provider** (local / OS / online), remote STT configuration as needed. **Remove** global “AI mode” toggle and Anthropic key fields (cleanup completed in T4-6 or in tandem).

**Acceptance criteria:**

- No Anthropic-specific settings surface in UI when T4-6 complete.
- STT choice persists across restart.
- `cargo test db::settings` passes after schema changes.

**Dependencies:** T4-3 types; coordination with T4-6 for deletion of legacy fields.

**Files:** `db/settings.rs`, `SettingsPanel.tsx`, `settingsStore.ts`, `lib.rs` commands.

---

### Task T4-5: Wake path integration — `lib.rs` orchestrator [M]

**Description:** Same as original Phase 4 T4-5: wake thread, secondary PCM, `wake-detected` IPC, `is_paused` gate, hotkey unchanged.

**Acceptance criteria:** Unchanged from original T4-5.

**Dependencies:** T4-1 or T4-2, T4-4 (settings read).

**Files:** `lib.rs`, `audio/mod.rs`.

---

### Task T4-6: Legacy AI removal — Haiku, `ai_mode`, and related code [M]

**Description:**

Remove the **extra** AI stack: Anthropic Haiku client (`src/ai/` or equivalent), `run_ai_mode` / `ai_mode` on `CommandNode`, `global_ai_mode` in settings, executor branches that call HTTP LLM for preview/intent, `sub_prompt` requirements tied to `ai_mode`, Settings UI for Anthropic, and any IPC/HUD “thinking” states that existed only for LLM responses. **Transcription** remains the only ML/remote “AI” surface.

Include DB migration if columns (`ai_mode`, etc.) are dropped or deprecated; update seeds and editor forms (`NodeForm.logic.ts`, `types.ts`). Remove or rewrite tests that asserted Haiku/`ai_mode` behavior.

**Acceptance criteria:**

- `rg` / codebase search: no `claude-haiku`, `anthropic` API client for commands, or `ai_mode` executor paths (except brief comments in changelog if needed).
- `cargo test` and `npm test` green.
- Editor and HUD copy contain no “AI mode” for LLM; optional mention of **speech recognition** provider only.

**Verification:** `cargo clippy -- -D warnings`, `npm run lint`, targeted grep for removed symbols.

**Dependencies:** Prefer completing after T4-3/T4-4 so STT settings replace removed panels; can be parallel if merge conflicts managed.

**Files (likely):** `ai/mod.rs` (delete), `commands/executor.rs`, `db/models.rs`, `db/mod.rs`, `lib.rs`, `keychain.rs`, `SettingsPanel.tsx`, `NodeForm.logic.ts`, tests.

---

### Task T4-7: App index — scan, cache, fuzzy resolve [M]

**Description:** Same as original Phase 4 T4-7 (Windows scanner, SQLite cache, `resolve_app`, executor integration).

**Acceptance criteria:** Unchanged from original T4-7.

**Dependencies:** Phase 2 `rapidfuzz`.

---

### Task T4-8: End-to-end Phase 4 integration [M]

**Description:**

Wire wake, **transcription provider selection**, and app index. **HUD:** `wake-detected` badge; **no** LLM “Thinking…” phase. Tray tooltip reflects wake engine. Degradation matrix updated: remove rows about Anthropic key for command execution.

**Acceptance criteria:**

- Wake → listen → transcript → match → execute works with **local** STT.
- App index resolves friendly app names.
- Phase 1–3 E2E regressions pass.

**Dependencies:** T4-5, T4-6, T4-7, T4-3/4.

---

### Task T4-9: Quality gates + docs [S]

**Description:**

README Phase 4: Porcupine, OWW flag, **STT providers** (local default, OS, online API BYOK), app index — **not** Anthropic BYOK for commands. Scripts listed. Vitest for settings store (STT-related events, not `ai-thinking`).

**Acceptance criteria:** `cargo fmt`, `cargo clippy`, `npm run lint`, `npm run test`, `npm run tauri build` pass.

**Dependencies:** T4-8.

---

### Checkpoint A — Wake + STT foundations

- [ ] `WakeDetector` + Porcupine tests green
- [ ] OWW behind feature flag
- [ ] Transcription abstraction + local path wired; settings keys present
- [ ] No new Anthropic dependencies added in this phase (cleanup may still be pending)

### Checkpoint B — Integration-ready

- [ ] Wake thread + `wake-detected` in DevTools
- [ ] STT provider persists; local path verified manually
- [ ] Legacy AI removal (T4-6) complete or explicitly scheduled before release
- [ ] App index count > 0 on Windows dev machine
- [ ] `cargo clippy -- -D warnings` clean

### Checkpoint C — Phase 4 complete

- [ ] Wake word E2E works
- [ ] Transcription provider switch works (at least local + one other path or documented stub)
- [ ] `OpenApp("Name")` resolves via index where applicable
- [ ] Phase 1–3 checklist still passes
- [ ] README + human sign-off

---

## Risks and mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| OS STT differs per Windows/macOS version | Medium | Ship Windows first; document fallbacks; default local |
| Remote STT latency / privacy | Medium | Clear UI labels; timeout; local default |
| Removing `ai_mode` breaks existing user DBs | Medium | Migrations with safe defaults; editor hides removed fields |
| `windows-rs` complexity for app scanner | Medium | Multi-source fallback; document gaps |

---

## Later phases (out of scope)

- Phase 5: Code signing, auto-updater, installers.
- Optional semantic match (`fastembed`) — separate decision.

---

## References

- `references/SPEC.md` — update Open Questions if cloud/STT wording still mentions Haiku for commands.
- `plan.md` (Phase 1) — IPC baseline.
- Prior Phase 4 docs in git history — Haiku/`ai_mode` tasks **superseded** by this revision.
