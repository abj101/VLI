# Focused-app context results

Status: implemented, uncommitted, **not ship-ready**. Three permitted tuning rounds are complete. Focused accuracy and zero-false-action gates remain unmet.

## Baseline and final measurements

Bundled d1-2B-Q4_K_M.gguf; threshold 0.5; two-call decider. GPU and CPU evals ran sequentially. All final evals had zero infrastructure/schema errors. NONE actions means an action ran on an empty-expectation fixture (lower is better).

| Run and set | Tool accuracy | Exact accuracy | NONE actions | p50 (ms) | p95 (ms) |
|---|---:|---:|---:|---:|---:|
| [GPU baseline, unfocused (201)](logs/baseline-gpu.log) | 95.5% | 95.5% | 1 | 37 | 74 |
| [CPU baseline, unfocused (201)](logs/baseline-cpu.log) | 96.0% | 95.5% | 2 | 203 | 421 |
| [GPU final, app+title: unfocused (201)](logs/final-gpu-title.log) | 95.5% | 95.5% | 1 | 38 | 75 |
| [GPU final, app+title: focused (41)](logs/final-gpu-title.log) | 87.8% | 87.8% | 3 | 51 | 62 |
| [GPU final, app-only: unfocused (201)](logs/final-gpu-app.log) | 95.5% | 95.5% | 1 | 37 | 74 |
| [GPU final, app-only: focused (41)](logs/final-gpu-app.log) | 85.4% | 85.4% | 3 | 49 | 60 |
| [CPU final, app+title: unfocused (201)](logs/final-cpu-title.log) | 96.0% | 95.5% | 2 | 204 | 345 |
| [CPU final, app+title: focused (41)](logs/final-cpu-title.log) | 90.2% | 90.2% | 2 | 223 | 251 |
| [CPU final, app-only: unfocused (201)](logs/final-cpu-app.log) | 96.0% | 95.5% | 2 | 203 | 326 |
| [CPU final, app-only: focused (41)](logs/final-cpu-app.log) | 87.8% | 87.8% | 2 | 228 | 249 |

The retained candidate is round two with **app+title**. App-only is 2.4 percentage points worse on GPU (85.4% versus 87.8%) and CPU (87.8% versus 90.2%), exceeding the brief's one-point cutoff. Titles remain in capture/context. **No variant is claimed shipped.**

Latency and unfocused gates pass on both devices. Focused accuracy and zero-false-action gates fail on both.

## Fixtures and tuning

Original 201 entries remain unfocused. Their complete raw JSON prefix was verified byte-for-byte against the preserved baseline executable. Added 41 focused entries, including 12 NONE entries and 10 Spotify chatter entries. Fixtures were written before model results and were not changed to fit them.

| Candidate | GPU unfocused exact | Unfocused NONE actions | GPU focused app+title exact | Focused NONE actions | Disposition |
|---|---:|---:|---:|---:|---|
| [Initial context](logs/focus-gpu-title.log) | 95.5% | 1 | 75.6% | 5 | Before tuning |
| [Round 1](logs/tune1-gpu-title.log) | 94.5% | 2 | 80.5% | 2 | Rejected: unfocused regression |
| [Round 2](logs/tune2-gpu-title.log) | 95.5% | 1 | 87.8% | 3 | Retained |
| [Round 3](logs/tune3-gpu-title.log) | 95.5% | 2 | 90.2% | 3 | Rejected: extra unfocused NONE action |

Round one tried global media descriptions, enum hints, and context wording. Round two restored original tool descriptions and unfocused hints, limited extra enum wording to requests with context, and clarified the context sentence. Round three tried small global shortcut/media description changes and close hints; it was reverted because it increased unfocused NONE actions.

No per-phrase routing rules, app routing tables, threshold changes, or model-output-driven fixture edits were added. Tuning stopped after three rounds.

## Remaining failures by cause

Retained app+title candidate: five focused GPU failures, four focused CPU failures.

- **Tab/window confusion:** “move to the tab on the right” in Visual Studio Code selects snap_window(right_half), rather than shortcut(next_tab), on GPU and CPU.
- **Media controls despite browser context:** “skip to the next song” in Chrome on a news page and “make it louder” in Chrome on a code-review page run media actions; fixtures expect NONE, on GPU and CPU. These explicit media instructions expose the remaining semantic question of whether app context should suppress global media controls. The fixed fixtures require suppression.
- **Ambiguous dismissal rejected:** “get rid of this” in File Explorer falls below confidence on the shortcut enum, on GPU and CPU.
- **Reminiscence interpreted as playback control:** “this music takes me back” in Spotify runs media(previous) on GPU; CPU rejects it correctly.

The 201 no-context fixtures retain baseline-level failures: ambiguous clipboard/dictation instructions; undo/redo, tab and misspelled-track commands rejected at confidence; a multi-clause desktop-folder request mapped to show-desktop; and an incomplete close request executing. CPU also executes the song-reminds-me-of-summer NONE fixture, as in its baseline. Raw logs list every failure.

## Verification

- Baseline cargo test --lib: **456 passed, 3 failed, 7 ignored**. Existing clipboard access/contention and localhost HTTP tests failed in the sandbox. [Log](logs/baseline-lib.log).
- Final cargo test: **468 passed, 0 failed, 7 ignored**; binary/doc targets clean. Ran with clipboard/localhost access. [Log](logs/final-cargo-test.log).
- Final cargo clippy: **26 existing library warnings, no new source warnings**. Incremental caching disabled to remove sandbox hard-link warnings. Initial clippy observation was 30 while partial capture exports were not yet consumed, plus cache noise; final check has none of those transient diagnostics. [Log](logs/final-clippy.log).
- npx --no-install tsc --noEmit: passed using the installed compiler. [Log](logs/focus-tsc.log).
- npm test: **22 frontend files / 144 tests passed**, plus **49 composer checks passed / 1 ignored**. [Log](logs/focus-npm-test.log).
- Coverage includes index hit/miss, Unicode title truncation, quote/newline stripping, equal app/title omission, own-PID exclusion, both inference calls/every clause receiving context, exact legacy state with None, focus clearing on dismissal, and title-redacting Debug.
- npm run tauri dev launched with model/sidecar downloads disabled via an empty fetch-set selection. Hidden startup windows yielded no targetable Jarvis window in the desktop tools. **Chrome/Spotify/editor manual voice checks were not completed.** Live model fixtures cover the Chrome and Spotify phrases but do not verify end-to-end audio/hotkey/action behavior. Only the verified development process tree started here was stopped afterward. [Log](logs/focus-tauri-dev.log).
- Initial sandboxed eval failed its localhost sidecar health probe despite the model reporting ready. Successful baselines used the saved unchanged test executable with localhost access. That temporary executable was removed after fixture verification.

## Implementation and privacy

Capture snapshots the foreground executable/title before showing the HUD. Own-process windows receive no context; non-Windows capture returns None. Dismissal and dictation clear the snapshot. App names resolve from the same in-memory index used by tier zero, with executable-stem fallback. Only tier two receives context; the editor command passes None.

Titles are stripped of quotes/newlines and limited to 60 Unicode characters. Focus has no serialization support and redacts titles in Debug. Context goes only to the local systemone request. Learned-command saving still receives transcript and selected calls without a context/focus field. App log/error paths do not receive titles or context. Eval failure labels include only the app name and strip window titles. **No title is written to application logs, error messages, or the learned-command DB by this change.** Fixture titles are synthetic.

## Files changed by this task

Paths below are relative to jarvis/src-tauri/src:

- lib.rs: listening snapshot, session resets, dismissal assertion.
- commands/router.rs: app-index context construction before the worker; tier-two parameter.
- llm/decider.rs: context threading through both calls/all clauses, focused enum hints, tests.
- llm/focus_context.rs: pure formatter and tests.
- llm/mod.rs: focus-context module export.
- llm/tauri_cmds.rs: editor routing passes None.
- llm/router_eval.rs: optional fixture focus, separate tables, app-only switch, title-free failure labels. Legacy evaluators retain only unfocused fixtures.
- llm/router_eval_cases.json: append 41 focused fixtures.
- window/mod.rs: shared focus type, redacting Debug, exports and regression test.
- window/placement_windows.rs: foreground capture and own-PID test.
- window/placement_macos.rs and window/placement_stub.rs: None-returning capture.
- This report and raw verification/eval logs under docs/focus-context-results.

Existing dirty d1, settings, app-resolution and UI changes were preserved. Experimental db/tools.rs tuning edits were restored to their pre-task contents. No dependencies, downloads, commits, pushes, branch switches or staging operations were performed.
