//! Remote HTTP STT worker — see `mod.rs` for the JSON contract.

use base64::Engine;
use log::debug;
use serde::Serialize;
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

use crate::audio::stt::{
    resample_mono_to_16k, SilenceResetTracker, TranscriptUpdate, INFER_EVERY, MIN_DECODE_SAMPLES,
    TARGET_RATE,
};
use crate::audio::transcript_event::{
    audio_has_speech, classify_partial, normalize_transcript_candidate, TranscriptPartialKind,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct RemoteSttParams {
    pub endpoint: String,
    pub model: Option<String>,
    pub bearer_token: Option<String>,
    pub timeout: Duration,
}

fn f32_pcm_to_s16le_bytes(pcm: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(pcm.len() * 2);
    for &s in pcm {
        let x = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

#[derive(Serialize)]
struct RemoteAudioBody<'a> {
    encoding: &'static str,
    sample_rate_hz: u32,
    channels: u32,
    data: &'a str,
}

#[derive(Serialize)]
struct RemoteRequestBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt: Option<&'a str>,
    audio: RemoteAudioBody<'a>,
}

fn parse_transcript_from_response(body: &str) -> Result<String, String> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("remote STT: invalid JSON ({e})"))?;
    if let Some(t) = v.get("text").and_then(|x| x.as_str()) {
        return Ok(t.trim().to_string());
    }
    if let Some(t) = v.get("transcript").and_then(|x| x.as_str()) {
        return Ok(t.trim().to_string());
    }
    if let Some(t) = v.pointer("/result/text").and_then(|x| x.as_str()) {
        return Ok(t.trim().to_string());
    }
    Err("remote STT: response missing text/transcript".into())
}

fn post_transcription(
    params: &RemoteSttParams,
    pcm_16k: &[f32],
    prompt: Option<&str>,
) -> Result<String, String> {
    if pcm_16k.len() < MIN_DECODE_SAMPLES || !audio_has_speech(pcm_16k) {
        return Ok(String::new());
    }
    let raw = f32_pcm_to_s16le_bytes(pcm_16k);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&raw);
    let body = RemoteRequestBody {
        model: params.model.as_deref(),
        prompt,
        audio: RemoteAudioBody {
            encoding: "pcm_s16le",
            sample_rate_hz: TARGET_RATE,
            channels: 1,
            data: &b64,
        },
    };
    let json_body = serde_json::to_string(&body).map_err(|e| e.to_string())?;

    let mut req = ureq::post(&params.endpoint)
        .set("Content-Type", "application/json")
        .timeout(params.timeout);

    if let Some(token) = &params.bearer_token {
        if !token.is_empty() {
            req = req.set("Authorization", &format!("Bearer {token}"));
        }
    }

    let resp = req
        .send_string(&json_body)
        .map_err(|e| format!("remote STT: request failed ({e})"))?;

    if !(200..300).contains(&resp.status()) {
        return Err(format!(
            "remote STT: HTTP {} — {}",
            resp.status(),
            resp.status_text()
        ));
    }

    let text = resp
        .into_string()
        .map_err(|e| format!("remote STT: read body ({e})"))?;
    parse_transcript_from_response(&text)
}

pub fn spawn_remote_stt_thread(
    app: AppHandle,
    params: RemoteSttParams,
    pcm_rx: Receiver<Vec<f32>>,
    input_sample_rate: u32,
    hud_session_id: u64,
    for_dictation: bool,
) -> JoinHandle<()> {
    let app_err = app.clone();
    std::thread::spawn(move || {
        if let Err(e) = remote_stt_loop(
            app,
            params,
            pcm_rx,
            input_sample_rate,
            hud_session_id,
            for_dictation,
        ) {
            let _ = app_err.emit("audio-error", serde_json::json!({ "message": e }));
        }
    })
}

fn remote_stt_loop(
    app: AppHandle,
    params: RemoteSttParams,
    pcm_rx: Receiver<Vec<f32>>,
    input_sample_rate: u32,
    hud_session_id: u64,
    for_dictation: bool,
) -> Result<(), String> {
    if for_dictation {
        return remote_dictation_loop(&app, &params, pcm_rx, input_sample_rate, hud_session_id);
    }
    use std::time::Instant;

    let mut buffer_16k: Vec<f32> = Vec::new();
    let mut last_infer = Instant::now() - INFER_EVERY;
    let mut last_text = String::new();
    let mut silence_reset = SilenceResetTracker::for_mode(for_dictation);
    let mut consecutive_errors: u32 = 0;

    while let Ok(chunk) = pcm_rx.recv() {
        let chunk_16k = resample_mono_to_16k(&chunk, input_sample_rate);
        if silence_reset.push_and_should_reset(&chunk_16k) {
            buffer_16k.clear();
            if !last_text.is_empty() {
                last_text.clear();
                let _ = app.emit(
                    "transcript-update",
                    TranscriptUpdate {
                        text: String::new(),
                        is_final: false,
                        hud_session_id,
                        kind: TranscriptPartialKind::SilenceReset,
                    },
                );
            } else {
                last_text.clear();
            }
            continue;
        }
        buffer_16k.extend_from_slice(&chunk_16k);
        let cap = TARGET_RATE as usize * 4;
        if buffer_16k.len() > cap {
            let excess = buffer_16k.len() - cap;
            buffer_16k.drain(0..excess);
        }

        if last_infer.elapsed() < INFER_EVERY {
            continue;
        }
        last_infer = Instant::now();

        let text = match post_transcription(&params, &buffer_16k, None) {
            Ok(t) => {
                consecutive_errors = 0;
                t
            }
            Err(e) => {
                // Never log bearer token or raw HTTP bodies.
                debug!("remote STT inference error (session {hud_session_id}): {e}");
                consecutive_errors = consecutive_errors.saturating_add(1);
                if consecutive_errors == 1 || consecutive_errors.is_multiple_of(5) {
                    let _ = app.emit("audio-error", serde_json::json!({ "message": e }));
                }
                if consecutive_errors >= 3 {
                    std::thread::sleep(Duration::from_secs(2));
                }
                continue;
            }
        };

        let text = match normalize_transcript_candidate(&text) {
            Some(t) => t,
            None => {
                if !last_text.is_empty() {
                    last_text.clear();
                    let _ = app.emit(
                        "transcript-update",
                        TranscriptUpdate {
                            text: String::new(),
                            is_final: false,
                            hud_session_id,
                            kind: TranscriptPartialKind::SilenceReset,
                        },
                    );
                }
                continue;
            }
        };

        let Some(kind) = classify_partial(&last_text, &text) else {
            continue;
        };
        last_text = text.clone();
        let _ = app.emit(
            "transcript-update",
            TranscriptUpdate {
                text,
                is_final: false,
                hud_session_id,
                kind,
            },
        );
    }

    if buffer_16k.len() >= MIN_DECODE_SAMPLES {
        if let Ok(text) = post_transcription(&params, &buffer_16k, None) {
            if let Some(text) = normalize_transcript_candidate(&text) {
                let _ = app.emit(
                    "transcript-update",
                    TranscriptUpdate {
                        text,
                        is_final: true,
                        hud_session_id,
                        kind: TranscriptPartialKind::Growth,
                    },
                );
            }
        }
    }

    Ok(())
}

impl RemoteSttParams {
    /// Clamp user setting to a safe range; invalid values fall back to 30s.
    pub fn sanitized_timeout(secs: u32) -> Duration {
        if secs == 0 || secs > 300 {
            DEFAULT_TIMEOUT
        } else {
            Duration::from_secs(u64::from(secs))
        }
    }
}

fn remote_dictation_loop(
    app: &AppHandle,
    params: &RemoteSttParams,
    rx: Receiver<Vec<f32>>,
    rate: u32,
    id: u64,
) -> Result<(), String> {
    let mut buffer = Vec::new();
    let mut silence = SilenceResetTracker::for_mode(true);
    let end_on_pause = crate::load_dictation_hotkey_mode(app) == crate::DictationHotkeyMode::Toggle;
    let mut last_infer = std::time::Instant::now();
    let mut speech = false;
    let finalize = |buffer: &[f32]| {
        let started = std::time::Instant::now();
        let decoded = post_transcription(
            params,
            crate::audio::stt::trim_utterance(buffer),
            Some(&crate::dictation::dictation_prompt(app, id)),
        );
        if let Ok(raw) = decoded {
            if let Some(text) = normalize_transcript_candidate(&raw) {
                let _ = crate::dictation::publish_transcript(
                    app,
                    TranscriptUpdate {
                        text,
                        is_final: true,
                        hud_session_id: id,
                        kind: TranscriptPartialKind::Growth,
                    },
                    started.elapsed().as_millis(),
                );
            }
        }
    };
    while let Ok(chunk) = rx.recv() {
        let chunk = resample_mono_to_16k(&chunk, rate);
        speech |= audio_has_speech(&chunk);
        let pause = silence.push_and_should_reset(&chunk);
        crate::audio::stt::push_dictation(&mut buffer, &chunk);
        if (end_on_pause && speech && pause) || buffer.len() >= TARGET_RATE as usize * 28 {
            finalize(&buffer);
            buffer.clear();
            speech = false;
            last_infer = std::time::Instant::now();
            continue;
        }
        if last_infer.elapsed() >= INFER_EVERY {
            last_infer = std::time::Instant::now();
            let start = buffer.len().saturating_sub(TARGET_RATE as usize * 4);
            if let Ok(raw) = post_transcription(
                params,
                &buffer[start..],
                Some(&crate::dictation::dictation_prompt(app, id)),
            ) {
                if let Some(text) = normalize_transcript_candidate(&raw) {
                    let _ = crate::dictation::publish_transcript(
                        app,
                        TranscriptUpdate {
                            text,
                            is_final: false,
                            hud_session_id: id,
                            kind: TranscriptPartialKind::Growth,
                        },
                        0,
                    );
                }
            }
        }
    }
    finalize(&buffer);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_response_prefers_text() {
        let t = parse_transcript_from_response(r#"{"text":"  hello  "}"#).expect("ok");
        assert_eq!(t, "hello");
    }

    #[test]
    fn parse_response_accepts_transcript() {
        let t = parse_transcript_from_response(r#"{"transcript":"x"}"#).expect("ok");
        assert_eq!(t, "x");
    }

    #[test]
    fn parse_response_nested_result() {
        let t = parse_transcript_from_response(r#"{"result":{"text":"nested"}}"#).expect("ok");
        assert_eq!(t, "nested");
    }

    #[test]
    fn parse_response_errors_when_missing() {
        assert!(parse_transcript_from_response(r#"{}"#).is_err());
    }
}
