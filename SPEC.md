# Spec: JARVIS — Voice Command Automation Assistant

## Assumptions (correct if wrong)

1. **Repository layout (decided):** The Tauri + Vite app lives in **`jarvis/`** at the repo root (not next to `.git` directly). Paths are `jarvis/package.json`, `jarvis/src/`, `jarvis/src-tauri/`. Run npm/cargo dev commands **from `jarvis/`** unless CI wraps them.
2. **Platforms:** **Windows first** for MVP builds and acceptance; **macOS** is still in scope but may ship **after** Windows is solid. Linux is out of scope unless you add it later.
3. **Voice stack:** **Porcupine** (wake), **Whisper.cpp** (STT, `tiny.en`), **Piper** (TTS) as in BrainStorm; licensing may swap wake engine to **OpenWakeWord** later without changing the rest of the spec.
4. **Cloud:** Voice pipeline is **on-device only**; **Anthropic Claude Haiku** is used **only** when a command node has `ai_mode: true` (user supplies API key, never committed).
5. **Auth / accounts:** **Local-first** — no user accounts or cloud command sync in v1.
6. **Toolchain:** **Node LTS** + **Rust stable** (Tauri 2 requirements as documented at scaffold time).

---

## Objective

**What:** A cross-platform desktop assistant that listens for a **wake phrase** (or global hotkey), captures a **command phrase**, matches it to **command nodes** (exact or fuzzy), runs **ordered action chains** (apps, URLs, scripts, keys, TTS, wait, follow-ups), and shows a **minimal glass HUD** while listening and executing.

**Who:** Power users who want **hands-free, repeatable automations** without a heavy assistant or cloud voice by default.

**Why:** Small footprint (~target < 20MB binary, < 80MB idle RAM), privacy (local STT/TTS/wake), and **composable “Shortcuts-like”** command definitions stored in **SQLite**.

**Success looks like:**

- User can **wake** (hotkey in MVP; wake word in later phase), **speak a command**, see **live transcription** on the HUD, and have **matched actions run** within **< 100ms** after match (per BrainStorm).
- User can define **at least one** command node with **trigger phrase(s)** and **actions** (`open_app`, `open_url`, …) persisted in **SQLite** and surviving restart.
- **STT latency** for `tiny.en` toward **< 400ms** on a reference dev machine (tunable with hardware note in Open Questions).
- **HUD** follows the state machine in BrainStorm: `listening` → `matched` → `executing` → (`awaiting_input` if follow-up) → `done` / `stopped`.
- **System tray:** pause/resume and quit (MVP); editor entry as UI lands.

**Acceptance criteria (MVP slice — Phase 1 alignment):**

1. Tauri 2 app runs on **Windows** for MVP; macOS is targeted next once Windows MVP is stable (document any macOS gaps in release notes).
2. Global hotkey shows HUD; mic → Whisper → **transcript stream** visible on HUD.
3. **Exact-match** command phrase → **`open_app`** or **`open_url`** executes.
4. Commands stored in **SQLite** via Rust (`sqlx` or equivalent Tauri pattern).
5. Tray: **pause/resume** listening and **quit**.

---

## Tech Stack

| Layer | Choice |
|--------|--------|
| Shell | **Tauri 2** (Rust + system tray, global shortcuts, windows) |
| UI | **React 18**, **TypeScript**, **Vite** |
| UI motion / state | **Framer Motion**, **Zustand** |
| Wake word (later phase) | **Porcupine** (evaluate **OpenWakeWord** if licensing/custom phrase is blocked) |
| STT | **Whisper.cpp** (`tiny.en`, ~75MB model asset) |
| TTS | **Piper** (offline) |
| Fuzzy match | **rapidfuzz** (Rust side preferred for command matching) |
| Optional semantic match | **fastembed** + MiniLM-L3 (optional / later) |
| DB | **SQLite** (Rust, e.g. `sqlx`) |
| Optional AI follow-up | **Anthropic** `claude-haiku-4-5` — **only** when `ai_mode: true` |
| Bundling | Tauri bundler (`.exe` / `.dmg`), auto-updater in distribution phase |

---

## Commands

Run these **from `jarvis/`** (the app package root). Align script names with the scaffold when it exists.

```bash
cd jarvis

npm install

# Frontend dev server only
npm run dev

# Tauri app (desktop + webview)
npm run tauri dev

# Production build (platform installers via Tauri)
npm run tauri build

# Quality gates (add when configured)
npm run lint
npm run test
npm run format
```

```bash
cd jarvis/src-tauri

cargo test
cargo clippy -- -D warnings
cargo fmt
```

**CI (recommended once repo has code):** `cd jarvis` then the same npm/cargo checks; or configure workflow `defaults.run.working-directory: jarvis`.

---

## Project Structure

Repository layout (monorepo-style: app isolated under **`jarvis/`**):

```
VLI/                       # Git repo root (can hold SPEC.md, BrainStorm.md, .cursor/, future packages)
  SPEC.md
  BrainStorm.md
  jarvis/                  # Tauri + Vite application (all app code here)
    package.json
    vite.config.ts         # (or .js — per scaffold)
    src-tauri/
      src/
        audio/             # Wake word, STT, TTS pipelines
        commands/          # Node tree, matcher, executor
        db/                # SQLite schema, migrations, queries
        lib.rs / main.rs   # App entry, plugin wiring
      Cargo.toml
    src/
      components/
        HUD/               # Overlay: transcription, waveform, stop
        Editor/            # Node graph command builder (later phase)
        Settings/          # Theme, keys, thresholds (phased)
      store/               # Zustand stores
      main.tsx
    public/                # Static assets (if needed)
    tests/ or src/**/*.test.ts
```

**Docs at repo root:** `SPEC.md`, `BrainStorm.md`. Avoid extra markdown unless you ask for them.

---

## Code Style

**TypeScript / React:** explicit types on public APIs; functional components; colocate small hooks; prefer early returns; no `any` without comment.

```tsx
type HudPhase = "listening" | "matched" | "executing" | "awaiting_input" | "done" | "stopped";

type HudState = {
  phase: HudPhase;
  transcript: string;
  highlightSpan?: { start: number; end: number };
};

export function useHudState(initial: Pick<HudState, "phase">): HudState {
  const [state, setState] = React.useState<HudState>({
    transcript: "",
    ...initial,
  });
  return state;
}
```

**Rust:** `rustfmt` defaults; `snake_case` modules; `PascalCase` types; errors as `thiserror` or Tauri’s patterns; avoid `unwrap()` in non-test paths — use `?` or explicit handling.

**Naming:** Tauri commands: `verb_noun` (e.g. `start_listening`); DB tables: `snake_case` plural; React files: `PascalCase.tsx` for components.

**UI:** No emoji in builder/HUD per BrainStorm; geometric icons; shared **10px** radius / **44px** node height in editor when implemented.

---

## Testing Strategy

| Level | Scope | Tools (planned) |
|--------|--------|------------------|
| Unit (Rust) | Matcher, action executor, DB models | `cargo test` (from `jarvis/src-tauri`) |
| Unit (TS) | State machines, small pure helpers | **Vitest** |
| Integration | Tauri commands callable from tests where feasible | Rust integration tests + optional Vitest + mocked IPC |
| E2E | Critical flows (hotkey → HUD → action) | **Playwright** or similar **later** |

**Coverage:** pragmatic thresholds in CI once baseline exists (e.g. **70%+** on new pure logic modules); don’t block MVP on E2E if flaky on CI VMs without mic.

**Manual:** mic/STT/wake paths require **device checks** documented in a short checklist when needed.

---

## Boundaries

**Always**

- Run **format + lint + unit tests** before merge for touched areas.
- Keep **secrets and API keys** out of git (env / OS keychain / local config).
- Validate **IPC payloads** on Rust side for commands that touch filesystem or shell.
- Preserve **on-device** voice processing defaults; cloud only for explicit `ai_mode` + user key.

**Ask first**

- **New dependencies** (especially native/audio crates with license obligations).
- **Database schema** migrations that could lose user commands.
- **CI / signing / notarization** and **auto-updater** behavior.
- Dropping or **relaxing** performance / privacy targets in the spec.

**Never**

- Commit **API keys**, Porcupine access keys, or **personal paths** from dev machines.
- Remove or skip **failing tests** to “go green” without explicit approval.
- Ship **always-on cloud listening** for core voice without user opt-in and clear UI.

---

## Success Criteria (release-ready v1 — spans phases)

- [ ] User can create/edit **command nodes** (JSON or editor, whichever ships first) with triggers + actions + optional **sub_prompt** branches.
- [ ] **Fuzzy** matching works with **per-node** threshold (default **0.80**).
- [ ] **HUD** matches BrainStorm: dimensions, waveform circle, stop control, transcription + highlight behavior, minimal chrome.
- [ ] **Wake word** path OR documented **hotkey-only** mode for users who skip Porcupine.
- [ ] **Tray** supports editor, pause/resume, quit; **click-through** when idle as specified.
- [ ] **Installers** for Windows/macOS + **code signing** when Phase 5 is in scope.

---

## Open Questions

1. **Wake word:** Porcupine free tier vs **OpenWakeWord** — decision before Phase 4.
2. **STT fallback:** Optional cloud STT for weak hardware vs strict offline-only?
3. **Monetization:** Confirm “free core + paid/BYOK AI” only affects packaging/UX, not core architecture.
4. **Command sync:** Remains **local-only** until a future spec revision.
5. **Plugin system:** Deferred; keep **action executor** modular so new action types are additive.
6. **Repo name vs product name:** Workspace is **VLI**; product is **JARVIS** — confirm app **bundle ID** / window title naming.

---

## References

- Vision, UI, and phased roadmap: `BrainStorm.md` (Project Truth Document v0.1).

When this spec is **approved**, next gated step is **PLAN** (architecture + sequencing), then **TASKS**, then **IMPLEMENT** — per spec-driven-development workflow.
