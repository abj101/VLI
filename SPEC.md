# Spec: Windows app index — list + search (fix)

## Assumptions (correct me before implementation)

1. **Primary surface** is the command editor’s **Open app** action (`CommandFormulaRow` → `search_app_index` invoke), not a separate HUD-only flow unless you confirm HUD must match.
2. **“Show all apps”** means: when the picker opens with an **empty** query, the backend returns a **sorted, capped** slice of the full in-memory index (current behavior: first N by display name, limit clamped 1–200). Fixing “broken” includes ensuring the index is **populated** (scan + DB load) before search runs.
3. **Coverage** includes games (e.g. Steam/Epic entries), **system** and **store (UWP)** apps, and other launchable targets already described in `scanner_windows.rs` (registry, Start Menu, recursive exe scan, etc.). Perfect enumeration of every `.exe` on disk is not required if it conflicts with perf caps already in the scanner.
4. **Tests** for Counter-Strike 2, Notepad, and Discord use **deterministic fixtures** (mock `AppEntry` lists or small Rust test vectors) so CI does not depend on a specific Windows install. Optional **manual** verification on a dev machine remains useful but is not the gate.

## Objective

**Problem:** The installed-app picker / search is not behaving correctly (empty results, wrong results, or failed invoke).

**Goal:** Users can **browse** the app list (empty query → many apps, sorted) and **filter** by typing (substring on display name, path, and exe stem — and any agreed fuzzy/token fallback if we add it). The index should reflect **runnable Windows software**, including **games** (launcher-backed and others), **built-in/system** tools, and common third-party apps, consistent with the existing multi-source scanner.

**Users:** Jarvis users configuring voice/commands who pick an application to open.

**Success looks like:**

- Opening the app picker shows a **non-empty** list once the index has loaded (after first scan or cache load).
- Searching finds representative entries:
  - **Notepad** (built-in / accessory path).
  - **Discord** (typical Win32 install).
  - **Counter-Strike 2** (or equivalent Steam display string, e.g. “Counter-Strike 2” / CS2 — matched by substring rules).
- Automated tests assert search behavior for those **names** against **fixture data** (no flaky dependency on the host PC).

## Tech stack


| Layer          | Stack                                                                       |
| -------------- | --------------------------------------------------------------------------- |
| Desktop shell  | Tauri 2                                                                     |
| UI             | React 19, Vite 7, TypeScript                                                |
| Backend        | Rust (`jarvis/src-tauri`), `invoke` commands                                |
| App index      | In-memory `AppIndexStore` + SQLite cache (`db/app_index`)                   |
| Scanning       | `apps/scanner_windows.rs` (registry, Start Menu, Steam/Epic/GOG, UWP, etc.) |
| Frontend tests | Vitest                                                                      |
| Backend tests  | `cargo test` in `jarvis/src-tauri`                                          |


## Commands

Run from `jarvis/` unless noted.

```bash
# Install deps (once)
npm install

# Frontend unit tests (includes app invoke helpers)
npm test

# Rust unit tests (app resolve + DB + scanner-related tests)
cargo test --manifest-path src-tauri/Cargo.toml

# Typecheck + production build (sanity)
npm run build

# Dev (manual verification of picker)
npm run dev
# and in another terminal:
npm run tauri dev
```

Lint:

```bash
npm run lint
```

## Project structure (relevant)

```
jarvis/
  src/
    components/editor/
      CommandFormulaRow.tsx    # Open app picker, calls search_app_index
      appIndexInvoke.ts        # Payload shape for Tauri command
      appIndexInvoke.test.ts   # Invoke arg contract tests
    ...
  src-tauri/
    src/
      lib.rs                   # search_app_index command, AppIndexStore
      apps/
        mod.rs                 # AppEntry, resolve_app (fuzzy), scan_installed_apps
        scanner_windows.rs     # Windows enumeration
      db/app_index.rs          # SQLite persistence
```

Root `**SPEC.md**` (this file) is the feature spec; no separate `docs/` requirement for this fix.

## Code style

Match existing patterns: Tauri commands take a `payload` object where the Rust parameter is named `payload`; TypeScript uses `searchAppIndexInvokeArgs` so deserialization stays consistent.

**Example (TS — keep payload nesting):**

```ts
export function searchAppIndexInvokeArgs(query: string, limit: number) {
  return { payload: { query, limit } } as const;
}
```

**Example (Rust — naming and caps):**

- `AppEntry { display_name, exe_path, icon_data_url }` — serialize for frontend as today.
- Search: trim query; empty → sorted alphabetical slice; non-empty → filter with stable, documented rules; `limit` clamped (e.g. 1–200).

## Testing strategy


| Level           | What                                                                                                                                                                                               | Where                                                                         |
| --------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| **Unit (Rust)** | `search_app_index` filtering: empty query ordering; substring matches on name/path/stem; fixtures including **Notepad**, **Discord**, **Counter-Strike 2** (or “Counter-Strike 2” display string). | `lib.rs` (tests module) or `apps/mod.rs` / small `#[cfg(test)]` helper module |
| **Unit (TS)**   | `searchAppIndexInvokeArgs` remains correct for the command API.                                                                                                                                    | `appIndexInvoke.test.ts`                                                      |
| **Optional**    | If extraction is needed, pure Rust fn for “filter + sort” tested without spinning Tauri.                                                                                                           | Same as above                                                                 |


**Coverage expectation:** New tests must run in **CI without Windows-only data**; use **in-memory vectors** of `AppEntry` for the three product names.

**Not required for merge:** E2E driving the full Tauri window (unless you add a separate initiative).

## Boundaries

### Always do

- Preserve the `payload` wrapper for `search_app_index` from the frontend unless the backend is changed in lockstep.
- Keep **scan** and **search** concerns separate: scan fills the store; search only reads it.
- Add or update tests when changing search or index-loading behavior.

### Ask first

- Raising scanner **depth**, adding heavy new scan sources, or changing **24h cache** policy (perf + privacy).
- New npm/Rust dependencies.
- Changing the public shape of `AppEntry` or the `search_app_index` contract.

### Never do

- Ship **secrets** or machine-specific paths in tests as hardcoded “truth” for all developers.
- **Silently swallow** scan errors without logging; empty index should be diagnosable.
- Remove or weaken tests to hide flakiness — fix the root cause or use fixtures.

## Success criteria

1. **Browse:** Empty query returns up to `limit` entries, **sorted** by display name (then path), documented cap.
2. **Search:** Non-empty query returns entries whose **display name, full path, or exe stem** matches substring rules (document any extension, e.g. tokenization or fuzzy, if added).
3. **Representative apps:** Automated tests demonstrate hits for **Notepad**, **Discord**, and **Counter-Strike 2** (fixture `display_name` / `exe_path` strings agreed in test code).
4. **No regression:** `npm test` and `cargo test` for the touched crates pass in CI.

## Open questions

1. Should **HUD** (or any other surface) call `search_app_index` as well, or is editor-only sufficient?
2. Is **substring-only** search acceptable long-term, or should we add **fuzzy** ranking for the picker (while keeping `resolve_app` behavior for voice distinct if needed)?
3. What exactly is “broken” today: **empty list**, **invoke error**, **wrong ordering**, or **missing specific apps** on your machine? Short answer helps scope the first fix.

---

**Next step:** Review this spec; answer **Open questions** where you can. After approval, phase 2 is a short implementation plan and task breakdown, then implementation + tests.