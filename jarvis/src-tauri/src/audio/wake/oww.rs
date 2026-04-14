//! OpenWakeWord ONNX backend (`ort`): melspectrogram + embedding + wake-word classifier (Apache-2.0 models from upstream releases).
//!
//! Models are **not** committed; fetch with `npm run fetch-wake-models` (from `jarvis/`) or `scripts/download-oww-model.ps1` into `resource_dir/oww/`.

use crate::audio::wake::{WakeDetector, WakeError};
use log::warn;
use ndarray::{concatenate, s, Array2, Array3, Axis};
use ort::session::builder::SessionBuilder;
use ort::session::Session;
use ort::value::TensorRef;
use std::collections::VecDeque;
use std::path::Path;

const SAMPLE_RATE: usize = 16_000;
const CHUNK_SAMPLES: usize = 1280; // 80 ms @ 16 kHz
const MEL_BINS: usize = 32;
const EMB_WINDOW_ROWS: usize = 76;
const MELSPEC_BUFFER_MAX_ROWS: usize = 10 * 97;
const FEATURE_BUFFER_MAX_ROWS: usize = 120;
const RAW_BUFFER_MAX_SAMPLES: usize = SAMPLE_RATE * 10;

const MELSPEC_ONNX: &str = "melspectrogram.onnx";
const EMBEDDING_ONNX: &str = "embedding_model.onnx";
const WAKE_ONNX: &str = "hey_jarvis_v0.1.onnx";

fn ort_map(e: ort::Error) -> WakeError {
    WakeError::Process(format!("{e}"))
}

/// ONNX + streaming preprocessor for OpenWakeWord (hey_jarvis). Threshold matches upstream `predict` (default 0.5 in settings).
pub struct OpenWakeWordBackend {
    melspec: Session,
    embedding: Session,
    wakeword: Session,
    wakeword_input: String,
    /// Second dimension of wake ONNX input (typically 16 for hey_jarvis).
    model_input_frames: usize,
    threshold: f32,
    pre: OwwPreprocessor,
    pcm_pending: Vec<i16>,
}

struct OwwPreprocessor {
    raw_data_buffer: VecDeque<i16>,
    melspectrogram_buffer: Array2<f32>,
    accumulated_samples: usize,
    raw_data_remainder: Vec<i16>,
    feature_buffer: Array2<f32>,
}

impl OwwPreprocessor {
    fn new() -> Self {
        Self {
            raw_data_buffer: VecDeque::with_capacity(RAW_BUFFER_MAX_SAMPLES),
            melspectrogram_buffer: Array2::ones((EMB_WINDOW_ROWS, MEL_BINS)),
            accumulated_samples: 0,
            raw_data_remainder: Vec::new(),
            feature_buffer: Array2::zeros((16, 96)),
        }
    }

    fn buffer_raw(&mut self, x: &[i16]) {
        for &s in x {
            if self.raw_data_buffer.len() >= RAW_BUFFER_MAX_SAMPLES {
                self.raw_data_buffer.pop_front();
            }
            self.raw_data_buffer.push_back(s);
        }
    }

    /// Port of `AudioFeatures._streaming_features` (openWakeWord `utils.py`).
    fn streaming_features(
        &mut self,
        x: &[i16],
        melspec: &mut Session,
        embedding: &mut Session,
    ) -> Result<usize, WakeError> {
        let mut x = x.to_vec();
        if !self.raw_data_remainder.is_empty() {
            let mut merged = std::mem::take(&mut self.raw_data_remainder);
            merged.extend_from_slice(&x);
            x = merged;
        }

        let mut processed_samples = 0usize;

        if self.accumulated_samples + x.len() >= CHUNK_SAMPLES {
            let remainder = (self.accumulated_samples + x.len()) % CHUNK_SAMPLES;
            if remainder != 0 {
                let split = x.len() - remainder;
                let x_even = &x[..split];
                self.buffer_raw(x_even);
                self.accumulated_samples += x_even.len();
                self.raw_data_remainder = x[split..].to_vec();
            } else {
                self.buffer_raw(&x);
                self.accumulated_samples += x.len();
                self.raw_data_remainder.clear();
            }
        } else {
            self.accumulated_samples += x.len();
            self.buffer_raw(&x);
        }

        if self.accumulated_samples >= CHUNK_SAMPLES
            && self.accumulated_samples.is_multiple_of(CHUNK_SAMPLES)
        {
            self.streaming_melspectrogram(self.accumulated_samples, melspec)?;
            let n_chunks = self.accumulated_samples / CHUNK_SAMPLES;
            let buf_rows = self.melspectrogram_buffer.nrows() as isize;
            for i in (0..n_chunks as isize).rev() {
                let mut ndx = -8 * i;
                if ndx == 0 {
                    ndx = buf_rows;
                }
                let window = mel_window_76(&self.melspectrogram_buffer, ndx)?;
                let emb = embedding_predict(embedding, &window)?;
                self.feature_buffer =
                    concatenate(Axis(0), &[self.feature_buffer.view(), emb.view()])
                        .map_err(|e| WakeError::Process(format!("feature vstack: {e}")))?;
            }
            processed_samples = self.accumulated_samples;
            self.accumulated_samples = 0;
        }

        if self.feature_buffer.nrows() > FEATURE_BUFFER_MAX_ROWS {
            let start = self.feature_buffer.nrows() - FEATURE_BUFFER_MAX_ROWS;
            self.feature_buffer = self.feature_buffer.slice(s![start.., ..]).to_owned();
        }

        Ok(if processed_samples != 0 {
            processed_samples
        } else {
            self.accumulated_samples
        })
    }

    fn streaming_melspectrogram(
        &mut self,
        n_samples: usize,
        melspec: &mut Session,
    ) -> Result<(), WakeError> {
        if self.raw_data_buffer.len() < 400 {
            return Err(WakeError::Process(
                "openWakeWord preprocessor: raw buffer too small (<400 samples)".into(),
            ));
        }
        let take = n_samples + 160 * 3;
        let buf: Vec<i16> = self
            .raw_data_buffer
            .iter()
            .rev()
            .take(take)
            .copied()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let spec = get_melspectrogram(melspec, &buf)?;
        self.melspectrogram_buffer =
            concatenate(Axis(0), &[self.melspectrogram_buffer.view(), spec.view()])
                .map_err(|e| WakeError::Process(format!("mel vstack: {e}")))?;
        if self.melspectrogram_buffer.nrows() > MELSPEC_BUFFER_MAX_ROWS {
            let start = self.melspectrogram_buffer.nrows() - MELSPEC_BUFFER_MAX_ROWS;
            self.melspectrogram_buffer =
                self.melspectrogram_buffer.slice(s![start.., ..]).to_owned();
        }
        Ok(())
    }

    fn get_features(&self, n_frames: usize) -> Result<Array3<f32>, WakeError> {
        let n = self.feature_buffer.nrows();
        if n < n_frames {
            return Err(WakeError::Process(format!(
                "openWakeWord: need at least {n_frames} feature rows, have {n}"
            )));
        }
        let slice = self.feature_buffer.slice(s![n - n_frames.., ..]);
        Ok(slice.to_owned().insert_axis(Axis(0)))
    }
}

/// `melspectrogram_buffer[-76+ndx:ndx]` (NumPy rules for negative indices).
fn mel_window_76(buf: &Array2<f32>, ndx: isize) -> Result<Array2<f32>, WakeError> {
    let r = buf.nrows() as isize;
    let mut start = -76 + ndx;
    let mut end = ndx;
    if end <= 0 {
        end += r;
    }
    if start <= 0 {
        start += r;
    }
    if end - start != 76 {
        return Err(WakeError::Process(format!(
            "mel window slice expected 76 rows, got {} (ndx={ndx}, r={r})",
            end - start
        )));
    }
    Ok(buf.slice(s![start as usize..end as usize, ..]).to_owned())
}

fn get_melspectrogram(session: &mut Session, x: &[i16]) -> Result<Array2<f32>, WakeError> {
    let f: Vec<f32> = x.iter().map(|&s| s as f32).collect();
    let input = Array2::from_shape_vec((1, x.len()), f)
        .map_err(|e| WakeError::Process(format!("mel: {e}")))?;
    let outputs = session
        .run(ort::inputs!["input" => TensorRef::from_array_view(input.view()).map_err(ort_map)?])
        .map_err(ort_map)?;
    let v = &outputs[0];
    let arr = v
        .try_extract_array::<f32>()
        .map_err(|e| WakeError::Process(format!("mel extract: {e}")))?;
    let mut spec = arr.to_owned();
    while spec.ndim() > 2 {
        spec = spec.index_axis_move(Axis(0), 0);
    }
    let mut spec2 = spec
        .into_dimensionality::<ndarray::Ix2>()
        .map_err(|e| WakeError::Process(format!("mel rank: {e}")))?;
    spec2.mapv_inplace(|v| v / 10.0 + 2.0);
    Ok(spec2)
}

fn embedding_predict(
    session: &mut Session,
    window76: &Array2<f32>,
) -> Result<Array2<f32>, WakeError> {
    let batch = window76.view().insert_axis(Axis(0)).insert_axis(Axis(3));
    let outputs = session
        .run(ort::inputs!["input_1" => TensorRef::from_array_view(batch).map_err(ort_map)?])
        .map_err(ort_map)?;
    let v = &outputs[0];
    let arr = v
        .try_extract_array::<f32>()
        .map_err(|e| WakeError::Process(format!("embedding extract: {e}")))?;
    let flat = arr.to_owned();
    match flat.ndim() {
        2 => flat
            .into_dimensionality()
            .map_err(|e| WakeError::Process(format!("emb: {e}"))),
        3 => {
            let d = flat.into_dimensionality::<ndarray::Ix3>().unwrap();
            Ok(d.index_axis_move(Axis(0), 0))
        }
        n => Err(WakeError::Process(format!("embedding rank {n}"))),
    }
}

impl OpenWakeWordBackend {
    /// Loads ONNX models from `resource_dir/oww/`. `threshold` is compared to the classifier score (settings default 0.5).
    pub fn try_new(resource_dir: &Path, threshold: f32) -> Result<Self, WakeError> {
        let dir = resource_dir.join("oww");
        let melspec_path = dir.join(MELSPEC_ONNX);
        let emb_path = dir.join(EMBEDDING_ONNX);
        let wake_path = dir.join(WAKE_ONNX);
        if !melspec_path.is_file() || !emb_path.is_file() || !wake_path.is_file() {
            warn!(
                "OpenWakeWord: missing ONNX under {}; oww backend unavailable",
                dir.display()
            );
            return Err(WakeError::Init(format!(
                "missing OpenWakeWord models under {} (run `npm run fetch-wake-models` from jarvis/ or scripts/download-oww-model.ps1)",
                dir.display()
            )));
        }

        let melspec = session_from_file(&melspec_path)?;
        let embedding = session_from_file(&emb_path)?;
        let wakeword = session_from_file(&wake_path)?;

        let w_in = wakeword
            .inputs()
            .first()
            .map(|o| o.name().to_string())
            .ok_or_else(|| WakeError::Init("hey_jarvis model has no inputs".into()))?;
        let model_input_frames = wakeword
            .inputs()
            .first()
            .and_then(|o| o.dtype().tensor_shape())
            .and_then(|sh| sh.get(1).copied())
            .filter(|&d| d > 0)
            .map(|d| d as usize)
            .unwrap_or(16);

        Ok(Self {
            melspec,
            embedding,
            wakeword,
            wakeword_input: w_in,
            model_input_frames,
            threshold,
            pre: OwwPreprocessor::new(),
            pcm_pending: Vec::new(),
        })
    }

    /// Test-only: classifier score threshold from settings / `try_new`.
    #[cfg(test)]
    pub(crate) fn threshold(&self) -> f32 {
        self.threshold
    }

    fn run_wakeword(&mut self) -> Result<f32, WakeError> {
        let input: Array3<f32> = self.pre.get_features(self.model_input_frames)?;
        let name = self.wakeword_input.as_str();
        let outputs = self
            .wakeword
            .run(ort::inputs![name => TensorRef::from_array_view(input.view()).map_err(ort_map)?])
            .map_err(ort_map)?;
        let v = &outputs[0];
        let arr = v
            .try_extract_array::<f32>()
            .map_err(|e| WakeError::Process(format!("wakeword extract: {e}")))?;
        Ok(arr.iter().copied().next().unwrap_or(0.0))
    }
}

fn session_from_file(path: &Path) -> Result<Session, WakeError> {
    SessionBuilder::new()
        .map_err(|e| WakeError::Init(format!("ort SessionBuilder: {e}")))?
        .with_intra_threads(1)
        .map_err(|e| WakeError::Init(format!("ort intra threads: {e}")))?
        .with_inter_threads(1)
        .map_err(|e| WakeError::Init(format!("ort inter threads: {e}")))?
        .commit_from_file(path)
        .map_err(|e| WakeError::Init(format!("load onnx {}: {e}", path.display())))
}

impl WakeDetector for OpenWakeWordBackend {
    fn process_frame(&mut self, pcm: &[i16]) -> Result<bool, WakeError> {
        self.pcm_pending.extend_from_slice(pcm);
        let mut fired = false;
        while self.pcm_pending.len() >= CHUNK_SAMPLES {
            let chunk: Vec<i16> = self.pcm_pending.drain(..CHUNK_SAMPLES).collect();
            let n = self
                .pre
                .streaming_features(&chunk, &mut self.melspec, &mut self.embedding)?;
            if n == CHUNK_SAMPLES {
                let score = self.run_wakeword()?;
                if score >= self.threshold {
                    fired = true;
                }
            }
        }
        Ok(fired)
    }

    fn backend_name(&self) -> &'static str {
        "oww"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn oww_try_new_without_models_errors() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let r = OpenWakeWordBackend::try_new(tmp.path(), 0.5);
        assert!(r.is_err());
    }

    #[test]
    fn oww_backend_name_when_models_present() {
        let Some(dir) = std::env::var_os("JARVIS_OWW_MODEL_DIR") else {
            return;
        };
        let base = PathBuf::from(dir);
        if !base.join("oww").join(WAKE_ONNX).is_file() {
            return;
        }
        let b = OpenWakeWordBackend::try_new(&base, 0.5).expect("oww init");
        assert_eq!(b.backend_name(), "oww");
    }

    #[test]
    fn oww_silence_does_not_fire_when_models_present() {
        let Some(dir) = std::env::var_os("JARVIS_OWW_MODEL_DIR") else {
            return;
        };
        let base = PathBuf::from(dir);
        if !base.join("oww").join(WAKE_ONNX).is_file() {
            return;
        }
        let mut b = OpenWakeWordBackend::try_new(&base, 0.99).expect("oww init");
        for _ in 0..32 {
            assert!(!b.process_frame(&[0i16; CHUNK_SAMPLES]).expect("process"));
        }
    }

    #[test]
    fn oww_try_open_wake_word_oww_uses_app_settings_threshold() {
        use crate::db::AppSettings;
        let Some(dir) = std::env::var_os("JARVIS_OWW_MODEL_DIR") else {
            return;
        };
        let base = PathBuf::from(dir);
        if !base.join("oww").join(WAKE_ONNX).is_file() {
            return;
        }
        let settings = AppSettings {
            porcupine_key_stored: false,
            wake_engine: "oww".into(),
            oww_threshold: 0.73,
            stt_provider: "local".into(),
            remote_stt_url: String::new(),
            remote_stt_model: None,
            remote_stt_timeout_secs: 30,
            remote_stt_key_stored: false,
        };
        let b = crate::audio::wake::try_open_wake_word_oww(&base, &settings).expect("oww init");
        assert!((b.threshold() - 0.73).abs() < 0.000_1);
    }
}
