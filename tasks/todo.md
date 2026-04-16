# App Index Fix — TODO

Full plan: `[plan.md](./plan.md)`. Spec: `[../SPEC.md](../SPEC.md)`.

## Phase 1 — Scan reliability (landed)

- Task 1 — Reorder `scan()`, move COM+StartMenu to best-effort last pass. `scanner_windows.rs`.
- Task 2 — Preserve previous index when rescan returns empty. `lib.rs`.
- Task 3 — Extract `sorted_app_name_slice` + `filter_app_entries_substring`, add fixture tests for Notepad/Discord/CS2. `apps/mod.rs` + `lib.rs`.
- **Checkpoint 1 (human):** `npm run tauri dev` → picker lists many apps, finds Notepad/Discord, and at least one launcher game.

## Phase 2 — Widen coverage

- Task 4 — `scan_heroic` (Legendary + GOG + Amazon Nile + sideload). `scanner_windows.rs`.
- Task 5 — Resilience audit: only `scan_start_menu` returns `Result`; wrapped best-effort at call site. No `?` leaks out of `scan()`.
- Task 6 — `ScanStats` `log::info!` with per-source counts via `run_pass` wrapper.
- **Checkpoint 2:** `cargo test --lib` 147 passed, 0 failed. Manual: check the `app index scan stats:` log line on next run.

## Phase 3 — Frontend + polish

- Task 7 — `deriveAppSearchMeta` now takes `indexCount`; picker shows "Indexing apps…" vs `No apps match "<query>"`.
- Task 8 — `npm run lint` + `cargo build --lib` clean.
- **Checkpoint 3 (human):** `npm run tauri dev`, open the picker, verify Notepad + Discord + Steam/Epic/Heroic games appear and the empty-state copy reads correctly.

## Decisions

1. Heroic — include **Legendary + GOG + Amazon + sideloaded**.
2. Scan logging — `log::info!` line only.
3. Picker search — substring only.

