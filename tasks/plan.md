# Implementation Plan: Phase 4 scope revision — STT-only “AI,” legacy LLM removal

## Overview

Phase 4 delivery is reframed: **wake word**, **user-selectable transcription** (local on-device, OS-provided, or online API), and **app index** remain in scope. **Anthropic Haiku, per-command `ai_mode`, and the `src/ai/` LLM client** are out of scope and will be **removed** under a dedicated cleanup task. The only ML-related surface that stays is **speech-to-text** for the command phrase.

This plan aligns `references/Phase4Plan.md` and `references/Phase4Todo.md` with that direction and orders work by dependency (wake foundations → transcription settings → wake orchestration → legacy removal → app index → integration → quality).

## Architecture decisions

- **Single “AI” narrative:** User-facing “intelligence” is **transcription** (model/provider choice). Command interpretation stays **matcher + executor**, not an LLM.
- **Secrets:** API keys for **remote STT** live in the OS keychain (same pattern as Porcupine). No Anthropic service for commands.
- **Legacy removal (T4-6):** Scheduled after or alongside **T4-3/T4-4** so Settings already exposes STT options before Anthropic UI is deleted.

## Dependency graph (components)

```
audio/wake (WakeDetector)          audio/transcription (STT backends)
         \                                     /
          \                                   /
           ---- lib.rs orchestrator -----------
                         |
                    db/settings
                         |
              React Settings + editor (commands)
                         |
                    apps/ (index) ──► executor (OpenApp resolve)
```

- **Wake** and **STT** are siblings feeding `lib.rs`; neither imports the other’s internals.
- **Settings** depends on **DB schema**; UI depends on **IPC commands**.
- **T4-6 (cleanup)** touches DB, executor, `ai/`, editor, and stores — depends on knowing the replacement **STT** settings shape (T4-3/T4-4).

## Task list (vertical slices)

### Phase 1: Foundations

- [ ] **Task 1:** T4-1 / T4-2 wake backends — already largely implemented; finish any open items (e.g. `oww_threshold` wiring with T4-5).
- [ ] **Task 2:** T4-3 transcription abstraction + local default + settings contract for OS + remote.
- [ ] **Task 3:** T4-4 Settings IPC + UI — STT provider; Porcupine; wake engine; strip Anthropic/global AI mode (or pair with Task 4).

### Checkpoint: Foundations

- [ ] `cargo test` and `cargo clippy -- -D warnings` pass on touched Rust code.
- [ ] Local STT path still matches Phase 1–3 behavior when selected.
- [ ] Written settings for STT persist across restart.

### Phase 2: Integration and removal

- [ ] **Task 4:** T4-5 wake thread integration in `lib.rs` (`wake-detected`, pause gate).
- [ ] **Task 5:** T4-6 remove Haiku / `ai_mode` / `src/ai/` / Anthropic settings / executor LLM preview / related tests and types.
- [ ] **Task 6:** T4-7 app index (Windows scan, cache, `resolve_app`).

### Checkpoint: Integration-ready

- [ ] Wake triggers pipeline; `wake-detected` visible to frontend.
- [ ] No remaining command-time Anthropic calls; grep confirms.
- [ ] App index count emitted and used for `OpenApp`.

### Phase 3: E2E and ship

- [ ] **Task 7:** T4-8 HUD/tray/startup sequencing; degradation matrix without Anthropic rows.
- [ ] **Task 8:** T4-9 quality gates + README Phase 4 section (STT providers, not LLM commands).

### Checkpoint: Complete

- [ ] Phase 1–3 E2E checklist still passes.
- [ ] `npm run tauri build` succeeds.
- [ ] Human sign-off.

## Risks and mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| T4-6 migration drops columns in use | High | Back up DB contract; additive-first migrations; test on old DB file |
| OS STT API variance | Medium | Windows first; feature-gate; fall back to local |
| Large diff mixing STT + cleanup | Medium | Separate commits or PRs: settings + STT first, then removal |

## Open questions

- Exact **remote STT** HTTP API (OpenAI Whisper-compatible vs vendor-specific) — lock in T4-3 before UI fields.
- macOS **OS STT** timeline — stub acceptable for Phase 4 if Windows ships first.

## References

- `references/Phase4Plan.md` — full task specs T4-1 … T4-9.
- `references/Phase4Todo.md` — checkbox tracker.
- `references/SPEC.md` — may need a small edit to drop “Haiku for commands” if still stated.
