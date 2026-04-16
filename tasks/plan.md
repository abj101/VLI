# Implementation Plan: Windows app index — list + search (fix)

Source spec: `[SPEC.md](../SPEC.md)`

## Overview

The Windows installed-app picker in the command editor (`search_app_index`) currently shows only a handful of apps, misses games from Steam/Epic/Heroic, and fails to match Notepad/Discord by name. Fix the **indexing pipeline** so the store is reliably populated, and confirm the **search contract** (alphabetical capped slice when empty; substring match on name/path/exe stem) is preserved with deterministic fixture tests for **Notepad**, **Discord**, and **Counter-Strike 2**.

Per user direction: HUD is out of scope. Empty query = alphabetical capped slice. Fuzzy ranking stays out of the picker (we use substring); `resolve_app` fuzzy remains for voice-driven `OpenApp`.

## Architecture decisions

- **Scan order is load-bearing.** COM-dependent passes (Start Menu `.lnk` resolution) run **last** and are **best-effort**, so their failure cannot blank out the index.
- `**scan()` never returns `Ok(vec![])` when a previous index exists.** The background refresh preserves the last good in-memory + DB index when a scan yields zero entries.
- **Search behavior extracted into pure helpers** (`sorted_app_name_slice`, `filter_app_entries_substring`) so both the Tauri command and unit tests exercise the same code path without spinning Tauri.
- **Heroic** is new scope. Add a minimal scanner reading Heroic's installed-game JSON (`%APPDATA%\heroic\store_cache\`* / `%APPDATA%\heroic\GamesConfig`) behind its own function so a failure is isolated.
- **Fixture-based tests only** for product names (Notepad / Discord / CS2). Real-scan tests stay `#[ignore]` for manual/dev-machine runs.

## Dependency graph

```
AppEntry (apps/mod.rs)
  ├── scanner_windows::scan()           ← data producer
  │     ├── registry (uninstall, AppPaths)
  │     ├── launchers (Steam, Epic, GOG, Heroic)
  │     ├── UWP (Get-AppxPackage)
  │     ├── seed + recursive exe scan
  │     ├── Get-StartApps (PowerShell)
  │     └── COM + Start Menu .lnk (best-effort)
  │
  ├── AppIndexStore (lib.rs)            ← cache
  │     └── refresh_app_index_on_startup
  │             └── db::replace_app_index / load_app_index
  │
  ├── sorted_app_name_slice / filter_app_entries_substring (apps/mod.rs)
  │     └── search_app_index command (lib.rs)
  │             └── CommandFormulaRow.tsx → appIndexInvoke.ts
  │
  └── resolve_app (apps/mod.rs, unchanged) — used by voice-driven OpenApp
```

Build order follows bottom-up: scanner reliability → store preservation → search helpers/tests → (optional) Heroic → frontend sanity.

## Current status

Tasks 1, 2, 3 have already landed. Tasks 4–6 plus a new simplification pass (Tasks 9–11) landed this round:
- Inline PowerShell icon extraction is gone from the hot scan path — scans now complete in seconds instead of being dominated by N per-entry icon calls. Icons are fetched **lazily** via a new `get_app_icon` command.
- Steam/Epic/GOG/Heroic share a new `find_game_exe_in_tree` helper that scores candidates by display-name tokens / initials (so CS2 at `game/bin/win64/cs2.exe` and BG3 at `bin/bg3.exe` both resolve, and `crashhandler.exe` / `*_dx11.exe` variants lose).
- A new `scan_localappdata_apps` pass walks `%LOCALAPPDATA%\<App>\app-*\` so Squirrel installs (Discord, Teams classic, GitHub Desktop, 1Password) are indexed.
- The frontend now receives a `{ count, scanning }` payload on `app-index-ready`, exposes a `get_app_index_status` invoke for late-mounted UI, and surfaces a **Rescan now** button plus a live scanning indicator in Settings → About.

## Task list

### Phase 1 — Make the scan non-fatal and complete

#### Task 1: Reorder `scan()` so COM failure can't blank the index  — **DONE**

**Description:** Run registry → launchers (Steam/Epic/GOG) → UWP → accessory seed → recursive exe scan → `Get-StartApps`, then COM + `scan_start_menu` **last, best-effort** (log + continue).

**Acceptance criteria:**

- `scan()` returns a populated `Vec<AppEntry>` even if `CoInitializeEx` or `scan_start_menu` errors.
- Start Menu per-root failures are logged, do not abort sibling roots.

**Verification:**

- `cargo test --lib` passes (143/143 non-ignored).
- Manual: launch app, open picker, list is non-empty.

**Files:** `jarvis/src-tauri/src/apps/scanner_windows.rs`
**Scope:** S

---

#### Task 2: Preserve previous index on empty rescan  — **DONE**

**Description:** In `refresh_app_index_on_startup`, if the background scan returns zero entries but the in-memory store has entries, keep the previous list for both the store and the DB write.

**Acceptance criteria:**

- A rescan that returns `vec![]` does not overwrite a previously populated store/DB.
- Log warns when this fallback triggers.

**Verification:**

- `cargo test --lib` passes.
- Manual: temporarily simulate failure (e.g., unplug path) → picker still populated.

**Files:** `jarvis/src-tauri/src/lib.rs`
**Scope:** XS

---

#### Task 3: Extract search helpers + add fixture tests  — **DONE**

**Description:** Move the empty-query alphabetical slice and substring filter out of the Tauri command into `apps/mod.rs`; add `sorted_app_name_slice` and `filter_app_entries_substring`. Unit tests cover alphabetical cap and matches for **Notepad**, **Discord**, **Counter-Strike 2** + `cs2` stem.

**Acceptance criteria:**

- `search_app_index` delegates to helpers.
- Three fixture tests pass for Notepad / Discord / CS2.

**Verification:**

- `cargo test --lib apps::tests` passes.

**Files:** `jarvis/src-tauri/src/apps/mod.rs`, `jarvis/src-tauri/src/lib.rs`
**Scope:** S

---

### Checkpoint — Phase 1

- `cargo test --lib` — 143 passed, 0 failed.
- **Human run required:** `npm run tauri dev`, open the Open-app picker. Confirm many apps appear, and `notepad`, `discord`, plus a known Steam game all return hits.
- If the list is still small, capture `log` output before proceeding (helps target Phase 2).

---

### Phase 2 — Widen coverage (new scanners + resilience)

#### Task 4: Heroic Games Launcher scanner  — **DONE**

**Description:** Add `scan_heroic(&mut map)` alongside `scan_steam` / `scan_epic`. Read installed games from Heroic's config:

- Epic games Heroic tracks: `%APPDATA%\heroic\legendaryConfig\legendary\installed.json` (map entries include `title`, `install_path`, `executable`).
- GOG games Heroic tracks: `%APPDATA%\heroic\gog_store\installed.json` (fields: `appName`, `install_path`, `platform`) paired with `%APPDATA%\heroic\store_cache\gog_library.json` for display names.
- Amazon / sideload: `%APPDATA%\heroic\sideload_apps\library.json` (optional; skip if absent).

Ignore failures silently (each file is optional). Use `SourcePriority::Epic` for Heroic-Epic entries and `SourcePriority::Gog` for Heroic-GOG. Keep parsing lightweight (existing `json_str_field` helper if possible, or a small scoped helper).

**Acceptance criteria:**

- If `%APPDATA%\heroic` is absent, scanner is a no-op.
- When installed.json is present, entries are inserted with a valid `display_name` and an existing `.exe` path.
- Entries with non-existent exe paths are skipped.

**Verification:**

- New `#[cfg(test)]` parse tests against a small inline JSON fixture (no real Heroic install required).
- `cargo test --lib` passes.
- Manual (if Heroic installed): a known Heroic game appears in the picker.

**Files:**

- `jarvis/src-tauri/src/apps/scanner_windows.rs` (new `scan_heroic` + call-site in `scan()`)

**Scope:** M

---

#### Task 5: Harden each scanner against partial failure  — **DONE**

**Description:** Audit `scan_steam`, `scan_epic`, `scan_gog`, `scan_uwp`, `scan_get_start_apps`, `scan_program_files_recursive`, `seed_windows_accessories`, and the new `scan_heroic` to confirm none can `panic` or propagate errors out of `scan()`. Any IO, registry, or PowerShell error must be logged via `log::warn!` and the scanner must return normally. `scan_uwp` in particular calls PowerShell — confirm a PS failure (execution policy, missing `Get-AppxPackage`) cannot kill sibling passes.

**Acceptance criteria:**

- Every scanner returns `()` (not `Result`) or swallows `Result` at its entry point.
- All PowerShell invocations that can fail have an `Ok(o) if o.status.success() => ... _ => return` guard.
- No `?` inside `scan()` except inside `match` branches with explicit fallbacks.

**Verification:**

- `cargo test --lib` passes.
- Grep: no `?` on lines inside scanner functions that would leak to `scan()`'s result.

**Files:**

- `jarvis/src-tauri/src/apps/scanner_windows.rs`

**Scope:** S

---

#### Task 6: Diagnostic counters on scan  — **DONE**

**Description:** Add a lightweight `ScanStats { uninstall, app_paths, start_menu, start_apps, steam, epic, gog, heroic, uwp, accessory, exe_scan, total }` returned as `log::info!` (structured or simple key=value). Helps future debugging without building a UI. No new dependencies.

**Acceptance criteria:**

- One `log::info!` line per scan completion, with per-source counts and total.
- Counts are added without changing the public return type of `scan()`.

**Verification:**

- `cargo test --lib` passes.
- Manual: run app, observe the info line in Tauri dev logs.

**Files:**

- `jarvis/src-tauri/src/apps/scanner_windows.rs`

**Scope:** S

---

### Checkpoint — Phase 2

- `npm test` and `cargo test --lib` pass.
- Manual: logs show a `ScanStats` line; picker shows games from at least one launcher you have installed (Steam/Epic/GOG/Heroic).
- **Human review before Phase 3.**

---

### Phase 2.5 — Simplification: fast scan, deep coverage, honest UX

Triggered by user report: "Currently it just says indexing apps… in the dropdown", missing Discord / CS2 / BG3 / Notepad, settings count stuck.

#### Task 9: Defer icon extraction — **DONE**

**Description:** Remove every inline `extract_icon_data_url` call from `scan_uninstall_registry`, Start Menu `.lnk`, Steam/Epic/GOG/Heroic, and the accessory seed. Expose a new `get_app_icon` Tauri command + `apps::get_app_icon` helper that the UI calls lazily when a row is rendered / selected. Inline extraction spawned one PowerShell per entry (≈200–500 ms each), which was the real cause of the "Indexing apps…" hang.

**Acceptance criteria:**
- Every scanner passes `None` for icons into `insert_entry`.
- `get_app_icon(payload: { path })` returns an `Option<String>` data URL.
- Frontend fetches the icon on pick **and** on rehydrate of a previously-saved `open_app` action.

**Verification:** `cargo test --lib`, `npm test`, `npm run build` all green; 153/153 Rust tests.
**Files:** `apps/scanner_windows.rs`, `apps/mod.rs`, `lib.rs`, `CommandFormulaRow.tsx`.
**Scope:** M

---

#### Task 10: Deep game-exe discovery (`find_game_exe_in_tree`) — **DONE**

**Description:** Replace `install_dir_primary_exe` callsites in the launcher scanners with a scoring walker that recurses up to 4–5 levels, ranks each `.exe` by (a) full-name / token / initials-with-digit-suffix overlap, (b) graphics-API variant penalties (`*_dx11`, `*_vulkan`, `*_x64`), (c) junk penalties (`crash`, `report`, `handler`, `redist`, `anticheat`, …), (d) distance from the install root. Prunes `_CommonRedist` / `EngineRuntimes` / `EasyAntiCheat` subtrees so the scan stays cheap.

**Acceptance criteria:**
- `find_game_exe_in_tree(root, "Counter-Strike 2", 5)` resolves `game/bin/win64/cs2.exe` over `crashhandler.exe`.
- `find_game_exe_in_tree(root, "Baldur's Gate 3", 4)` prefers `bin/bg3.exe` over `bin/bg3_dx11.exe` and `EasyAntiCheat_Setup.exe`.
- Tree with no matching candidate → `None`.

**Verification:** 4 new fixture tests in `apps::scanner_windows::tests` (cs2, bg3, crash-reporter noise, empty tree).
**Files:** `apps/scanner_windows.rs`.
**Scope:** M

---

#### Task 11: LocalAppData / Squirrel scanner (Discord family) — **DONE**

**Description:** Add `scan_localappdata_apps` that walks `%LOCALAPPDATA%` (top level only, skipping `Microsoft`, `Packages`, `Temp`, `NVIDIA`, `Programs`, etc.) and, for each candidate subdirectory, picks the newest `app-<semver>/` folder via `find_squirrel_app_exe`. This is the standard install shape for Discord, Discord PTB/Canary, Teams classic, GitHub Desktop, 1Password, and Slack. Falls back to a shallow `<App>\<App>.exe` lookup for apps that don't use Squirrel.

**Acceptance criteria:**
- `find_squirrel_app_exe` prefers the lexicographically-last `app-*` subdirectory (semver-ordered).
- Scanner skip-list keeps us out of the noisy Microsoft/UWP directories.
- Scanner is a no-op when `%LOCALAPPDATA%` is unset or missing.

**Verification:** 2 new fixture tests (squirrel-newest, skip-list characterisation) + end-to-end `npm test`.
**Files:** `apps/scanner_windows.rs`.
**Scope:** S

---

#### Task 12: Scan-status UX (`AppIndexScanning`, `get_app_index_status`, Rescan button) — **DONE**

**Description:** Introduce a `struct AppIndexScanning(AtomicBool)` managed by Tauri. The startup path and the new `rescan_app_index` command share a single `spawn_app_index_scan(app, store)` that flips the flag, preserves the previous index on empty rescans, emits `{ count, scanning }` at both ends, and logs elapsed time. Frontend mounts now pull `get_app_index_status` so a late-opened Settings panel isn't stuck on "…". `deriveAppSearchMeta` now takes `isScanning` and stops lying about "Indexing apps…" once a scan has completed.

**Acceptance criteria:**
- `app-index-ready` payload includes `scanning: boolean`.
- `get_app_index_status` returns `{ count, scanning }`.
- Settings → About shows live count + "(scanning…)" suffix + a **Rescan now** button.
- The picker dropdown stops showing "Indexing apps…" after a scan finishes, even if the result set is still empty (shows "No apps match `<query>`" instead).

**Verification:** Updated `formulaRow.logic.test.ts` covers the scan-done-but-empty case; `npm test` green.
**Files:** `lib.rs`, `store/settingsStore.ts`, `EditorRoot.tsx`, `components/Settings/SettingsPanel.tsx`, `components/editor/CommandFormulaRow.tsx`, `components/editor/formulaRow.logic.ts`, `components/editor/formulaRow.logic.test.ts`.
**Scope:** M

---

### Phase 3 — Frontend UX + polish

#### Task 7: Empty-state and error messaging in the picker

**Description:** In `CommandFormulaRow.tsx`, when `appHits.length === 0` and `appHasSearched` is true, show a clear message that distinguishes **"no index yet"** (subscribe to `APP_INDEX_READY_EVENT`) from **"no matches for this query"**. Respect current design patterns.

**Acceptance criteria:**

- If the index count (from `settingsStore.appIndexCount`) is 0, empty state says "Indexing apps…".
- Otherwise, empty state says "No apps match `<query>`".
- No change to the `search_app_index` payload shape.

**Verification:**

- `npm test` passes (including existing `appIndexInvoke.test.ts`).
- `npm run build` succeeds.
- Manual: launch before the first scan completes → see indexing state; after scan, type a gibberish string → see "no match".

**Files:**

- `jarvis/src/components/editor/CommandFormulaRow.tsx`

**Scope:** S

---

#### Task 8: Lint + typecheck pass

**Description:** Run `npm run lint` and `npm run build` to catch any collateral damage from Phases 1–3. Fix warnings introduced by our edits only.

**Acceptance criteria:**

- `npm run lint` clean for touched files.
- `npm run build` succeeds.

**Verification:**

- Both commands pass locally.

**Files:** N/A (sweep)
**Scope:** XS

---

### Checkpoint — Complete

- All acceptance criteria met.
- `cargo test --lib` green.
- `npm test`, `npm run build`, `npm run lint` green.
- Human smoke test: browse picker, find Notepad, Discord, and at least one Steam/Epic/Heroic game.

## Risks and mitigations


| Risk                                                                       | Impact                    | Mitigation                                                                                                                                   |
| -------------------------------------------------------------------------- | ------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| PowerShell `Get-AppxPackage` blocked by execution policy on target machine | Medium (UWP apps missing) | Task 5 guards the invocation; other sources still populate index.                                                                            |
| Heroic JSON schema varies by version                                       | Medium                    | Task 4 parses only well-known fields; missing fields cause skip, not error.                                                                  |
| Recursive exe scan is slow on large `Program Files`                        | Medium                    | Already depth-capped (≤6); background thread; not blocking UI. Leave as-is unless profiling shows regression.                                |
| Icon extraction via PowerShell per-entry is slow                           | Medium                    | Already only runs for real paths; if scans still feel slow, consider deferring icon extraction to first-use. **Out of scope for this plan.** |
| Large index (>1000 apps) slows substring filter in store                   | Low                       | Current filter is O(n) per keystroke; acceptable up to ~10k entries. Monitor.                                                                |


## Parallelization opportunities

- **Safe in parallel:** Tasks 4 (Heroic), 6 (ScanStats), 7 (Frontend empty state) are independent once Phase 1 lands.
- **Must be sequential:** Task 5 (resilience audit) should wait for Task 4 so the Heroic scanner is audited too.

## Open questions — resolved

1. **Heroic coverage:** Include **everything** — Legendary (Epic), GOG, Amazon, and `sideload_apps/library.json`.
2. **Scan logging:** `log::info!` line only. No Tauri event.
3. **Picker search:** Substring only. Voice `resolve_app` fuzzy remains untouched.

