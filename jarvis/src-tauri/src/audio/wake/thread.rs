//! Dedicated mic capture → 16 kHz i16 → [`super::WakeDetector`] (T4-5).

use crate::audio::capture;
use crate::audio::stt::resample_mono_to_16k;
use log::{info, warn};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

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
    engine: &str,
    settings: &crate::db::AppSettings,
    is_paused: Arc<AtomicBool>,
    on_wake: Arc<dyn Fn() + Send + Sync + 'static>,
) -> Result<(), String> {
    let detector = super::build_wake_detector(engine, resource_dir.as_path(), settings)?;
    let fixed = detector.fixed_input_frame_len();
    let label = detector.backend_name().to_string();
    info!("wake: spawning thread backend={label}");

    std::thread::Builder::new()
        .name("jarvis-wake".into())
        .spawn(move || {
            wake_thread_main(app, detector, fixed, label, is_paused, on_wake);
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn wake_thread_main(
    app: AppHandle,
    mut detector: Box<dyn super::WakeDetector>,
    fixed: Option<usize>,
    backend_label: String,
    is_paused: Arc<AtomicBool>,
    on_wake: Arc<dyn Fn() + Send + Sync + 'static>,
) {
    let (pcm_tx, pcm_rx) = std::sync::mpsc::channel();
    let (mut capture, sample_rate) = match capture::start_capture(app.clone(), pcm_tx) {
        Ok(x) => x,
        Err(e) => {
            warn!("wake: could not open mic ({e}); wake word disabled");
            return;
        }
    };

    let mut pending_i16: Vec<i16> = Vec::new();

    for chunk in pcm_rx {
        if is_paused.load(Ordering::Relaxed) {
            continue;
        }

        let resampled = resample_mono_to_16k(&chunk, sample_rate);

        match fixed {
            Some(len) => {
                extend_i16_from_f32(&mut pending_i16, &resampled);
                while pending_i16.len() >= len {
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

    capture.stop();
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
