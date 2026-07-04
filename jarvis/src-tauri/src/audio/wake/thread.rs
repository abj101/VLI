//! Dedicated mic capture → 16 kHz i16 → [`super::WakeDetector`] (T4-5).

use crate::audio::capture;
use crate::audio::stt::resample_mono_to_16k;
use log::{info, warn};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

const WAKE_RECV_TICK: Duration = Duration::from_millis(100);
/// Consecutive mic open failures before surfacing `audio-error` to the HUD (30 × 100 ms).
const WAKE_MIC_FAIL_EMIT_THRESHOLD: u32 = 30;
/// Back off wake mic retries when permission is permanently denied.
const WAKE_MIC_DENIED_BACKOFF: Duration = Duration::from_secs(5);

/// Handle to stop and join the wake worker (live reload when settings change).
pub struct WakeSupervisor {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl WakeSupervisor {
    pub fn shutdown(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.join.take() {
            let _ = h.join();
        }
    }
}

fn extend_i16_from_f32(out: &mut Vec<i16>, samples: &[f32]) {
    out.extend(
        samples
            .iter()
            .map(|&s| (s.clamp(-1.0, 1.0) * 32767.0) as i16),
    );
}

/// Spawns a background thread with its own mic stream for wake-word detection.
/// `on_wake` is invoked from this thread; dispatch UI work to the main thread inside the callback.
pub(crate) fn spawn_wake_thread(
    app: AppHandle,
    resource_dir: std::path::PathBuf,
    engine: String,
    settings: crate::db::AppSettings,
    is_paused: Arc<AtomicBool>,
    mic_suppressed: Arc<AtomicBool>,
    on_wake: Arc<dyn Fn() + Send + Sync + 'static>,
) -> Result<WakeSupervisor, String> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = Arc::clone(&stop);

    let join = std::thread::Builder::new()
        .name("jarvis-wake".into())
        .spawn(move || {
            let detector = match super::build_wake_detector(
                engine.as_str(),
                resource_dir.as_path(),
                &settings,
            ) {
                Ok(d) => d,
                Err(e) => {
                    warn!("wake: detector init failed ({e}); wake word disabled");
                    return;
                }
            };
            let fixed = detector.fixed_input_frame_len();
            let label = detector.backend_name().to_string();
            info!("wake: thread started backend={label}");
            wake_thread_main(
                app,
                detector,
                fixed,
                label,
                stop_thread,
                is_paused,
                mic_suppressed,
                on_wake,
            );
        })
        .map_err(|e| e.to_string())?;

    Ok(WakeSupervisor {
        stop,
        join: Some(join),
    })
}

#[allow(clippy::too_many_arguments)]
fn wake_thread_main(
    app: AppHandle,
    mut detector: Box<dyn super::WakeDetector>,
    fixed: Option<usize>,
    backend_label: String,
    stop: Arc<AtomicBool>,
    is_paused: Arc<AtomicBool>,
    mic_suppressed: Arc<AtomicBool>,
    on_wake: Arc<dyn Fn() + Send + Sync + 'static>,
) {
    struct WakeMic {
        session: capture::CaptureSession,
        pcm_rx: std::sync::mpsc::Receiver<Vec<f32>>,
        sample_rate: u32,
    }

    let mut mic: Option<WakeMic> = None;
    let mut pending_i16: Vec<i16> = Vec::new();
    let mut mic_open_failures: u32 = 0;
    let mut mic_error_emitted = false;
    let mut last_mic_error: Option<String> = None;

    let open_mic = |app: &AppHandle, last_err: &mut Option<String>| -> Option<WakeMic> {
        let (pcm_tx, pcm_rx) = std::sync::mpsc::channel();
        match capture::start_capture(app.clone(), pcm_tx, false) {
            Ok((session, sample_rate)) => {
                *last_err = None;
                Some(WakeMic {
                    session,
                    pcm_rx,
                    sample_rate,
                })
            }
            Err(e) => {
                *last_err = Some(e.clone());
                warn!("wake: could not open mic ({e}); retrying");
                None
            }
        }
    };

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        if mic_suppressed.load(Ordering::Relaxed) || is_paused.load(Ordering::Relaxed) {
            if mic.take().is_some() {
                pending_i16.clear();
            }
            mic_open_failures = 0;
            std::thread::sleep(WAKE_RECV_TICK);
            continue;
        }

        if mic.is_none() {
            mic = open_mic(&app, &mut last_mic_error);
            if mic.is_none() {
                mic_open_failures = mic_open_failures.saturating_add(1);
                if !mic_error_emitted && mic_open_failures >= WAKE_MIC_FAIL_EMIT_THRESHOLD {
                    if let Some(message) = last_mic_error.clone() {
                        mic_error_emitted = true;
                        let _ = app.emit(
                            "audio-error",
                            serde_json::json!({ "message": message }),
                        );
                    }
                }
                let backoff = if cfg!(target_os = "macos")
                    && crate::macos::microphone_permission_is_denied()
                {
                    WAKE_MIC_DENIED_BACKOFF
                } else {
                    WAKE_RECV_TICK
                };
                std::thread::sleep(backoff);
                continue;
            }
            mic_open_failures = 0;
        }

        let WakeMic {
            ref pcm_rx,
            sample_rate,
            ..
        } = mic.as_ref().expect("mic open");

        let chunk = match pcm_rx.recv_timeout(WAKE_RECV_TICK) {
            Ok(c) => c,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                mic = None;
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                continue;
            }
        };

        let resampled = resample_mono_to_16k(&chunk, *sample_rate);

        if let Some(preroll) = app.try_state::<crate::audio::WakePreroll>() {
            preroll.push_resampled_16k(&resampled);
        }

        match fixed {
            Some(len) => {
                extend_i16_from_f32(&mut pending_i16, &resampled);
                while pending_i16.len() >= len {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let hit = match detector.process_frame(&pending_i16[..len]) {
                        Ok(v) => v,
                        Err(e) => {
                            warn!("wake: process_frame ({backend_label}): {e}");
                            pending_i16.clear();
                            break;
                        }
                    };
                    pending_i16.drain(..len);
                    if hit {
                        let _ = app.emit("wake-detected", json!({ "backend": &backend_label }));
                        on_wake();
                    }
                }
            }
            None => {
                let chunk_i16: Vec<i16> = resampled
                    .iter()
                    .map(|&s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
                    .collect();
                let hit = match detector.process_frame(&chunk_i16) {
                    Ok(v) => v,
                    Err(e) => {
                        warn!("wake: process_frame ({backend_label}): {e}");
                        continue;
                    }
                };
                if hit {
                    let _ = app.emit("wake-detected", json!({ "backend": &backend_label }));
                    on_wake();
                }
            }
        }
    }

    if let Some(mut m) = mic.take() {
        m.session.stop();
    }
    info!("wake: thread exiting ({backend_label})");
}

#[cfg(test)]
mod tests {
    use super::extend_i16_from_f32;

    #[test]
    fn extend_i16_clamps_to_i16_range() {
        let mut v = Vec::new();
        extend_i16_from_f32(&mut v, &[0.0, 1.0, -1.0]);
        assert_eq!(v.len(), 3);
        assert_eq!(v[1], i16::MAX);
        assert_eq!(v[2], -32767);
    }
}
