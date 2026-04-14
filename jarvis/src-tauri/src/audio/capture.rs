//! WASAPI (via cpal) mic capture → mono `f32` PCM chunks + `amplitude-update` events (Task 4a).

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

const AMPLITUDE_EMIT_MIN_INTERVAL: Duration = Duration::from_millis(48);

/// Peak-of-window → roughly 0..1 for UI (quiet speech still visible).
pub fn normalized_amplitude(mono_f32: &[f32]) -> f32 {
    if mono_f32.is_empty() {
        return 0.0;
    }
    let peak = mono_f32
        .iter()
        .fold(0.0f32, |a, &s| a.max(s.abs()));
    (peak * 6.0).clamp(0.0, 1.0)
}

fn mono_mix_interleaved<T: cpal::Sample + cpal::SizedSample>(data: &[T], channels: usize) -> Vec<f32>
where
    f32: cpal::FromSample<T>,
{
    if channels == 0 || data.is_empty() {
        return Vec::new();
    }
    let frames = data.len() / channels;
    let mut out = Vec::with_capacity(frames);
    for f in 0..frames {
        let base = f * channels;
        let mut acc = 0.0f32;
        for c in 0..channels {
            acc += data[base + c].to_sample::<f32>();
        }
        out.push(acc / channels as f32);
    }
    out
}

/// Holds CPAL stream; [`CaptureSession::stop`] or drop ends capture.
pub struct CaptureSession {
    stream: Option<Stream>,
}

impl CaptureSession {
    /// No-op if already stopped.
    pub fn stop(&mut self) {
        if let Some(s) = self.stream.take() {
            drop(s);
        }
    }
}

impl Drop for CaptureSession {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Opens default input device, streams mono `f32` PCM to `pcm_tx`, emits `amplitude-update`.
///
/// Returns capture handle + nominal sample rate for downstream resampling to 16 kHz.
pub fn start_capture(app: AppHandle, pcm_tx: Sender<Vec<f32>>) -> Result<(CaptureSession, u32), String> {
    let host = cpal::default_host();
    let device = match host.default_input_device() {
        Some(d) => d,
        None => {
            let _ = app.emit(
                "audio-error",
                serde_json::json!({ "message": "no microphone input device found" }),
            );
            return Err("no microphone input device found".into());
        }
    };

    let supported = device
        .default_input_config()
        .map_err(|e| format!("failed to read default mic config: {e}"))?;
    let sample_format = supported.sample_format();
    let base_cfg: StreamConfig = supported.clone().into();
    let channels = base_cfg.channels as usize;
    let sample_rate = base_cfg.sample_rate.0;

    let last_amp_emit = Arc::new(Mutex::new(Instant::now() - AMPLITUDE_EMIT_MIN_INTERVAL));

    let stream = match sample_format {
        SampleFormat::F32 => build_stream::<f32>(&device, &base_cfg, channels, app, pcm_tx, last_amp_emit)?,
        SampleFormat::I16 => build_stream::<i16>(&device, &base_cfg, channels, app, pcm_tx, last_amp_emit)?,
        other => {
            return Err(format!("unsupported mic sample format `{other:?}`"));
        }
    };

    stream
        .play()
        .map_err(|e| format!("failed to start mic stream: {e}"))?;

    Ok((CaptureSession { stream: Some(stream) }, sample_rate))
}

fn build_stream<T>(
    device: &Device,
    cfg: &StreamConfig,
    channels: usize,
    app: AppHandle,
    pcm_tx: Sender<Vec<f32>>,
    last_amp_emit: Arc<Mutex<Instant>>,
) -> Result<Stream, String>
where
    T: cpal::Sample,
{
    let app_err = app.clone();
    let err_fn = move |err| {
        let _ = app_err.emit(
            "audio-error",
            serde_json::json!({ "message": format!("mic stream error: {err}") }),
        );
    };

    let app_amp = app.clone();
    let stream = device
        .build_input_stream(
            cfg,
            move |data: &[T], _| {
                let mono = mono_mix_interleaved(data, channels);
                if mono.is_empty() {
                    return;
                }

                let amp = normalized_amplitude(&mono);
                let emit_now = {
                    let mut last = last_amp_emit.lock().unwrap();
                    let now = Instant::now();
                    if now.duration_since(*last) >= AMPLITUDE_EMIT_MIN_INTERVAL {
                        *last = now;
                        true
                    } else {
                        false
                    }
                };
                if emit_now {
                    let _ = app_amp.emit(
                        "amplitude-update",
                        serde_json::json!({ "amplitude": amp as f64 }),
                    );
                }

                if pcm_tx.send(mono).is_err() {
                    // STT side disconnected; ignore.
                }
            },
            err_fn,
            None,
        )
        .map_err(|e| format!("failed to build mic stream: {e}"))?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::normalized_amplitude;

    #[test]
    fn amplitude_empty_is_zero() {
        assert_eq!(normalized_amplitude(&[]), 0.0);
    }

    #[test]
    fn amplitude_scales_peak_and_clamps() {
        assert!((normalized_amplitude(&[0.1]) - 0.6).abs() < 1e-5);
        assert_eq!(normalized_amplitude(&[1.0]), 1.0);
    }
}
