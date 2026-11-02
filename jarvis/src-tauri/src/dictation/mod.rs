//! Voice dictation: stream STT into the focused text field without HUD focus steal.

pub mod cleanup;
mod format;
mod session;

use crate::audio::{self, stt::TranscriptUpdate, SharedAudioPipeline};
use serde::Serialize;
pub use session::{DictationSessionController, HudSnapshot};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

pub struct DictationState {
    controller: Mutex<DictationSessionController>,
    insertion: Mutex<()>,
    snap_stop_session: AtomicU64,
}

impl Default for DictationState {
    fn default() -> Self {
        Self {
            controller: Mutex::new(DictationSessionController::default()),
            insertion: Mutex::new(()),
            snap_stop_session: AtomicU64::new(0),
        }
    }
}

impl DictationState {
    pub fn is_active(&self) -> bool {
        self.controller
            .lock()
            .map(|g| g.is_active())
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationPhasePayload {
    pub active: bool,
    pub session_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub waiting_for_input: bool,
}

fn emit_dictation_phase(app: &AppHandle, active: bool, session_id: u64, text: Option<String>) {
    let _ = app.emit(
        "dictation-phase",
        DictationPhasePayload {
            active,
            session_id,
            text,
            waiting_for_input: waiting_for_input(app),
        },
    );
}

fn hud_snapshot(hud: &crate::SharedHud) -> Result<HudSnapshot, String> {
    let s = hud.lock().map_err(|_| "hud state poisoned".to_string())?;
    Ok(HudSnapshot {
        visible: s.visible,
        phase: s.phase,
        overlay_mode: s.overlay_mode,
    })
}

fn with_controller<F, T>(app: &AppHandle, f: F) -> Result<T, String>
where
    F: FnOnce(&mut DictationSessionController) -> Result<T, String>,
{
    let state = app.state::<DictationState>();
    let mut guard = state
        .controller
        .lock()
        .map_err(|_| "dictation state poisoned".to_string())?;
    f(&mut guard)
}

pub fn accepts(app: &AppHandle, session_id: u64) -> bool {
    with_controller(app, |ctrl| Ok(ctrl.accepts(session_id))).unwrap_or(false)
}

pub fn is_active(app: &AppHandle) -> bool {
    app.try_state::<DictationState>()
        .is_some_and(|s| s.is_active())
}
pub fn waiting_for_input(app: &AppHandle) -> bool {
    with_controller(app, |ctrl| Ok(ctrl.waiting_for_input())).unwrap_or(false)
}

fn input_ready(app: &AppHandle, session_id: u64) -> bool {
    let ready = with_controller(app, |ctrl| {
        Ok(ctrl.accepts(session_id) && !ctrl.waiting_for_input())
    }).unwrap_or(false);
    #[cfg(windows)]
    { ready && crate::window::focused_text_rect().is_some() }
    #[cfg(not(windows))]
    { ready }
}

/// Called by the caret follower; microphone changes run on the UI thread.
#[cfg(windows)]
pub fn set_input_available(app: &AppHandle, session_id: u64, ready: bool) -> Result<(), String> {
    let changed = with_controller(app, |ctrl| {
        Ok(ctrl.accepts(session_id) && ctrl.set_input_ready(ready, std::time::Instant::now()))
    })?;
    if !changed { return Ok(()); }
    emit_dictation_phase(app, true, session_id, None);
    if !ready {
        crate::show_dictation_overlay(app, &app.state::<crate::SharedHud>(), session_id, false)?;
    } else {
        let focused = crate::window::focused_window();
        with_controller(app, |ctrl| {
            ctrl.set_context(cleanup::context_for_exe(
                focused.as_ref().map(|window| window.exe_path.as_str()).unwrap_or(""),
            ));
            Ok(())
        })?;
    }
    let app_main = app.clone();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        if accepts(&app_main, session_id) {
            let audio = app_main.state::<SharedAudioPipeline>();
            if ready {
                crate::try_start_listening_audio(&app_main, &audio, session_id, false, true);
            } else {
                audio::finish_shared_pipeline(&app_main, &audio);
                if let Some(suppressed) = app_main.try_state::<audio::WakeMicSuppressed>() {
                    suppressed.0.store(true, Ordering::SeqCst);
                }
            }
        }
        let _ = tx.send(());
    }).map_err(|e| e.to_string())?;
    rx.recv().map_err(|e| e.to_string())?;
    if ready {
        let app = app.clone();
        std::thread::spawn(move || {
            if let Err(err) = flush_pending(&app, session_id) {
                log::warn!("dictation pending insertion: {err}");
            }
        });
    }
    Ok(())
}

#[cfg(windows)]
pub fn waiting_expired(app: &AppHandle, session_id: u64) -> bool {
    with_controller(app, |ctrl| {
        Ok(ctrl.accepts(session_id) && ctrl.waiting_expired(std::time::Instant::now()))
    }).unwrap_or(false)
}

pub(crate) fn mark_snap_stop(app: &AppHandle, session_id: u64) {
    app.state::<DictationState>()
        .snap_stop_session
        .store(session_id, Ordering::Release);
}

pub(crate) fn stopped_by_snap(app: &AppHandle, session_id: u64) -> bool {
    app.state::<DictationState>()
        .snap_stop_session
        .load(Ordering::Acquire)
        == session_id
}

pub fn stop_dictation(
    app: &AppHandle,
    hud: &crate::SharedHud,
    audio: &SharedAudioPipeline,
) -> Result<(), String> {
    audio::finish_shared_pipeline(app, audio);
    let session_id = with_controller(app, |ctrl| Ok(ctrl.session_id()))?;
    finish_live_utterance(app, session_id)?;
    with_controller(app, |ctrl| { ctrl.stop(); Ok(()) })?;
    crate::hide_dictation_overlay(app, hud)?;
    emit_dictation_phase(app, false, session_id, None);
    Ok(())
}

pub fn start_dictation(
    app: &AppHandle,
    hud: &crate::SharedHud,
    audio: &SharedAudioPipeline,
) -> Result<u64, String> {
    let focused = crate::window::focused_window();
    audio::stop_shared_pipeline(app, audio);
    let snapshot = hud_snapshot(hud)?;
    // Started from the voice HUD ("start dictation"): shrink it into the orb instead of jumping.
    let morph = snapshot.visible
        && snapshot.overlay_mode == crate::hud::HudOverlayMode::Command
        && app
            .get_webview_window(crate::hud::HUD_WINDOW_LABEL)
            .is_some_and(|window| window.is_visible().unwrap_or(false));
    if snapshot.blocks_dictation() {
        crate::abort_command_hud_session(app, hud, audio)?;
        if hud_snapshot(hud)?.blocks_dictation() {
            return Err("cannot start dictation while voice HUD is active".into());
        }
    }

    let session_id = {
        let mut s = hud.lock().map_err(|_| "hud state poisoned".to_string())?;
        let snapshot = HudSnapshot {
            visible: s.visible,
            phase: s.phase,
            overlay_mode: s.overlay_mode,
        };
        let session_id = s.session_id.wrapping_add(1);
        with_controller(app, |ctrl| {
            ctrl.start(session_id, snapshot)?;
            ctrl.set_context(cleanup::context_for_exe(
                focused
                    .as_ref()
                    .map(|window| window.exe_path.as_str())
                    .unwrap_or(""),
            ));
            Ok(())
        })?;
        s.session_id = session_id;
        session_id
    };

    if let Some(suppressed) = app.try_state::<audio::WakeMicSuppressed>() {
        use std::sync::atomic::Ordering;
        suppressed.0.store(true, Ordering::SeqCst);
    }

    #[cfg(windows)]
    let ready = crate::window::focused_text_rect().is_some();
    #[cfg(not(windows))]
    let ready = true;
    with_controller(app, |ctrl| {
        ctrl.set_input_ready(ready, std::time::Instant::now());
        Ok(())
    })?;
    // Publish waiting state before the HUD opens so no listening orb flashes at the center.
    emit_dictation_phase(app, true, session_id, None);
    if ready {
        crate::try_start_listening_audio(app, audio, session_id, false, true);
    }
    // Only Windows animates the morph; elsewhere the overlay keeps its usual geometry.
    crate::show_dictation_overlay(app, hud, session_id, morph && cfg!(windows))?;
    #[cfg(windows)]
    crate::hud::follow_text_box(app.clone(), session_id, morph || !ready);
    emit_dictation_phase(app, true, session_id, None);
    Ok(session_id)
}

pub fn toggle_dictation(
    app: &AppHandle,
    hud: &crate::SharedHud,
    audio: &SharedAudioPipeline,
) -> Result<(), String> {
    if is_active(app) {
        stop_dictation(app, hud, audio)
    } else {
        start_dictation(app, hud, audio)?;
        Ok(())
    }
}

pub fn dictation_prompt(app: &AppHandle, session_id: u64) -> String {
    let tail = with_controller(app, |ctrl| {
        Ok(if ctrl.accepts(session_id) {
            ctrl.tail().to_string()
        } else {
            String::new()
        })
    })
    .unwrap_or_default();
    let vocabulary = crate::open_db_connection(app)
        .ok()
        .and_then(|conn| crate::db::get_app_settings(&conn).ok())
        .map(|s| s.dictation_vocabulary)
        .unwrap_or_default();
    format_prompt(&vocabulary, &tail)
}

fn format_prompt(vocabulary: &str, tail: &str) -> String {
    let words = vocabulary
        .split(',')
        .map(str::trim)
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    if words.is_empty() {
        tail.to_string()
    } else if tail.is_empty() {
        format!("Vocabulary: {words}.")
    } else {
        format!("Vocabulary: {words}. {tail}")
    }
}

/// Dispatch directly for shutdown ordering, then notify webviews without re-entering the native listener.
pub fn publish_transcript(
    app: &AppHandle,
    update: TranscriptUpdate,
    decode_ms: u128,
) -> Result<(), String> {
    handle_transcript_timed(app, &update, decode_ms)?;
    app.emit_filter("transcript-update", update, |target| {
        !matches!(target, tauri::EventTarget::App)
    })
    .map_err(|e| e.to_string())
}

pub fn handle_transcript(app: &AppHandle, update: &TranscriptUpdate) -> Result<(), String> {
    handle_transcript_timed(app, update, 0)
}

fn handle_transcript_timed(
    app: &AppHandle,
    update: &TranscriptUpdate,
    decode_ms: u128,
) -> Result<(), String> {
    let (accepted, context) = with_controller(app, |ctrl| {
        Ok((ctrl.accepts(update.hud_session_id), ctrl.context()))
    })?;
    if !accepted {
        return Ok(());
    }
    let url_mode = url_mode(app, update.hud_session_id)?;
    let cleanup_started = std::time::Instant::now();
    let mut text = update.text.clone();
    if update.is_final && !url_mode && !format::has_hook(&text) {
        if let Ok(settings) = crate::open_db_connection(app)
            .and_then(|conn| crate::db::get_app_settings(&conn).map_err(|e| e.to_string()))
        {
            if settings.dictation_cleanup {
                text = cleanup::cleanup(
                    &text,
                    &settings.dictation_style,
                    context,
                    settings.local_whisper_use_gpu,
                );
            }
        }
    }
    let cleanup_ms = cleanup_started.elapsed().as_millis();
    if update.is_final {
        text = with_controller(app, |ctrl| {
            if !ctrl.accepts(update.hud_session_id) {
                return Ok(String::new());
            }
            let mut formatted = ctrl.format_words(&text, url_mode);
            let pending = format::flush(&mut ctrl.format_state, url_mode);
            if formatted.is_empty() {
                formatted = pending;
            } else {
                formatted.push_str(&session::join_and_capitalize(
                    &formatted, &pending, url_mode,
                ));
            }
            let result = ctrl
                .final_text(update.hud_session_id, &formatted, true)
                .unwrap_or_default();
            ctrl.url_mode = None;
            Ok(result)
        })?;
    }
    let paste_started = std::time::Instant::now();
    if !update.is_final {
        emit_dictation_phase(app, true, update.hud_session_id, Some(text.clone()));
    }
    if dispatch_final(update.is_final, &text, &mut |text| insert_text(app, update.hud_session_id, text))? {
        emit_dictation_phase(app, true, update.hud_session_id, Some(text.clone()));
    }
    if update.is_final {
        log::debug!(
            "dictation: decode {decode_ms} ms, cleanup {cleanup_ms} ms, paste {} ms",
            paste_started.elapsed().as_millis()
        );
    }
    Ok(())
}

/// Inserts words committed mid-utterance as soon as they are stable. No cleanup pass: text
/// already in the field can't be rewritten.
pub fn commit_live(app: &AppHandle, session_id: u64, words: &str) -> Result<(), String> {
    let url_mode = url_mode(app, session_id)?;
    let text = with_controller(app, |ctrl| {
        if !ctrl.accepts(session_id) {
            return Ok(String::new());
        }
        let text = ctrl.format_words(words, url_mode);
        Ok(ctrl.final_text(session_id, &text, true).unwrap_or_default())
    })?;
    if text.is_empty() {
        return Ok(());
    }
    insert_text(app, session_id, &text)?;
    // The HUD animates these characters into the head.
    emit_dictation_phase(app, true, session_id, Some(text));
    Ok(())
}

/// Address-bar check, once per utterance. The UIA call is cross-process and can be slow, so it
/// runs outside the controller lock (the HUD follower polls that lock every frame).
fn url_mode(app: &AppHandle, session_id: u64) -> Result<bool, String> {
    if let Some(cached) = with_controller(app, |ctrl| Ok(ctrl.url_mode))? {
        return Ok(cached);
    }
    let detected = crate::window::focused_is_address_bar();
    with_controller(app, |ctrl| {
        if ctrl.accepts(session_id) {
            ctrl.url_mode = Some(detected);
        }
        Ok(detected)
    })
}

/// The STT final branch signals this even when there are no remaining words.
pub fn finish_live_utterance(app: &AppHandle, session_id: u64) -> Result<(), String> {
    let text = with_controller(app, |ctrl| {
        Ok(if ctrl.accepts(session_id) {
            ctrl.finish_utterance()
        } else {
            String::new()
        })
    })?;
    if !text.is_empty() {
        insert_text(app, session_id, &text)?;
        emit_dictation_phase(app, true, session_id, Some(text));
    }
    Ok(())
}

/// Longest one insert may take to type, so long ones still land quickly.
const TYPE_MAX_MS: u32 = 1_500;

/// Per-character delays (ms) that read as live typing: ~50 chars/s with a little jitter, a beat
/// after spaces and longer after punctuation; scaled down to fit `TYPE_MAX_MS`.
fn typing_schedule(text: &str) -> Vec<u32> {
    let raw: Vec<u32> = text
        .chars()
        .enumerate()
        .map(|(i, ch)| {
            let base = match ch {
                '.' | '!' | '?' => 90,
                ',' | ';' | ':' => 55,
                ' ' => 28,
                _ => 18,
            };
            // Deterministic +-30% jitter so the rhythm feels human, not metronomic.
            let jitter = (i as u32).wrapping_mul(2_654_435_761) >> 26; // 0..63
            base * (70 + jitter) / 100
        })
        .collect();
    let total: u32 = raw.iter().sum();
    if total <= TYPE_MAX_MS {
        return raw;
    }
    raw.iter().map(|d| d * TYPE_MAX_MS / total).collect()
}

/// Queue accepted words before typing so losing focus cannot drop the untyped suffix.
fn insert_text(app: &AppHandle, session_id: u64, text: &str) -> Result<(), String> {
    with_controller(app, |ctrl| {
        if ctrl.accepts(session_id) { ctrl.queue_insert(text); }
        Ok(())
    })?;
    flush_pending(app, session_id)
}

fn flush_pending(app: &AppHandle, session_id: u64) -> Result<(), String> {
    let state = app.state::<DictationState>();
    let _insertion = state.insertion.lock().map_err(|_| "dictation insertion poisoned")?;
    if !input_ready(app, session_id) { return Ok(()); }
    let text = with_controller(app, |ctrl| {
        Ok(if ctrl.accepts(session_id) { ctrl.take_pending_insert() } else { String::new() })
    })?;
    let (inserted, result) = type_live(app, session_id, &text);
    if inserted < text.len() {
        with_controller(app, |ctrl| {
            if ctrl.accepts(session_id) {
                ctrl.pending_insert.insert_str(0, &text[inserted..]);
            }
            Ok(())
        })?;
    }
    result
}

/// Return the inserted UTF-8 prefix length so any remainder can wait for text focus.
fn type_live(app: &AppHandle, session_id: u64, text: &str) -> (usize, Result<(), String>) {
    crate::hud::set_dictation_typing(true);
    let mut result = Ok(());
    let mut inserted = 0;
    for ((i, ch), delay) in text.char_indices().zip(typing_schedule(text)) {
        // A lone "new line" inside a list continues it, numbered from the field's own text.
        let list_line = (ch == '\n' && continues_list(text, i))
            .then(crate::window::focused_line_text)
            .flatten();
        crate::input::type_text::wait_for_modifiers_released();
        // Focus may disappear while characters are being animated. Keep the suffix queued.
        if !input_ready(app, session_id) { break; }
        if crate::input::type_text::type_unicode(ch.encode_utf8(&mut [0; 4])).is_err() {
            result = paste(&text[i..]);
            if result.is_ok() { inserted = text.len(); }
            break;
        }
        inserted = i + ch.len_utf8();
        if let Some(marker) = list_line.as_deref().and_then(format::next_marker) {
            // Rich editors (Claude, Slack, Word) number the new item themselves.
            std::thread::sleep(std::time::Duration::from_millis(40));
            if input_ready(app, session_id)
                && !crate::window::focused_line_text().is_some_and(|line| format::has_marker(&line)) {
                crate::input::type_text::wait_for_modifiers_released();
                let _ = crate::input::type_text::type_unicode(&marker);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(delay as u64));
    }
    crate::hud::set_dictation_typing(false);
    (inserted, result)
}

/// The newline at `i` is a single "new line" (not a paragraph break or a spoken list start).
fn continues_list(text: &str, i: usize) -> bool {
    let after = &text[i + 1..];
    !text[..i].ends_with('\n') && !after.starts_with('\n') && !format::has_marker(after)
}

/// Inserts a sentence in one clipboard paste (fastest, and one undo step), restoring the clipboard.
fn paste(text: &str) -> Result<(), String> {
    let previous = crate::commands::executor::get_clipboard_text().ok();
    let result = crate::commands::executor::set_clipboard_text(text).and_then(|_| {
        crate::input::type_text::wait_for_modifiers_released();
        crate::input::send_keys::send_keys("^v")
    });
    if result.is_ok() {
        std::thread::sleep(std::time::Duration::from_millis(120));
    }
    if let Some(previous) = previous {
        if previous.trim().is_empty() {
            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                let _ = clipboard.set_text(previous);
            }
        } else {
            let _ = crate::commands::executor::set_clipboard_text(&previous);
        }
    }
    result
}

fn dispatch_final(
    is_final: bool,
    text: &str,
    insert: &mut impl FnMut(&str) -> Result<(), String>,
) -> Result<bool, String> {
    if !is_final || text.is_empty() {
        return Ok(false);
    }
    insert(text)?;
    Ok(true)
}

/// Stop dictation when HUD opens for command listening.
pub fn stop_for_hud(app: &AppHandle, hud: &crate::SharedHud, audio: &SharedAudioPipeline) {
    if is_active(app) {
        let _ = stop_dictation(app, hud, audio);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prompt_combines_vocabulary_and_tail() {
        assert_eq!(
            format_prompt("Claude, Jarvis, Tauri", ""),
            "Vocabulary: Claude, Jarvis, Tauri."
        );
        assert_eq!(format_prompt("", "Hello."), "Hello.");
        assert_eq!(
            format_prompt("Claude", "Hello."),
            "Vocabulary: Claude. Hello."
        );
    }
    #[test]
    fn typing_schedule_pauses_on_punctuation_and_caps_total() {
        let d = typing_schedule("Hi, there. Ok");
        assert_eq!(d.len(), 13);
        assert!(d[2] > d[1] && d[9] > d[1]); // punctuation pauses longer than letters
        assert!(typing_schedule(&"word ".repeat(200)).iter().sum::<u32>() <= TYPE_MAX_MS);
    }

    #[test]
    fn partials_never_insert_final_inserts_once() {
        let mut calls = Vec::new();
        let mut insert = |text: &str| {
            calls.push(text.to_owned());
            Ok(())
        };
        dispatch_final(false, "hel", &mut insert).unwrap();
        dispatch_final(false, "hello world", &mut insert).unwrap();
        dispatch_final(true, "hello world", &mut insert).unwrap();
        assert_eq!(calls, vec!["hello world"]);
    }
}
