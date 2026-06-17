//! Rolling wake-mic buffer at 16 kHz — seeds the listen STT pipeline after "hey jarvis".

use std::sync::{Arc, Mutex};

/// ~1.5 s of mono 16 kHz audio retained from the wake thread.
pub const WAKE_PREROLL_MAX_SAMPLES: usize = 16_000 + 8_000;

#[derive(Clone, Default)]
pub struct WakePreroll(pub Arc<Mutex<WakePrerollBuffer>>);

#[derive(Debug, Default)]
pub struct WakePrerollBuffer {
    samples: Vec<f32>,
}

impl WakePrerollBuffer {
    pub fn push_resampled_16k(&mut self, chunk: &[f32]) {
        if chunk.is_empty() {
            return;
        }
        self.samples.extend_from_slice(chunk);
        let excess = self.samples.len().saturating_sub(WAKE_PREROLL_MAX_SAMPLES);
        if excess > 0 {
            self.samples.drain(0..excess);
        }
    }

    pub fn take_snapshot(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.samples)
    }
}

impl WakePreroll {
    pub fn push_resampled_16k(&self, chunk: &[f32]) {
        if let Ok(mut g) = self.0.lock() {
            g.push_resampled_16k(chunk);
        }
    }

    pub fn take_snapshot(&self) -> Vec<f32> {
        self.0
            .lock()
            .map(|mut g| g.take_snapshot())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preroll_ring_caps_at_max_samples() {
        let mut buf = WakePrerollBuffer::default();
        let chunk = vec![0.1f32; WAKE_PREROLL_MAX_SAMPLES / 2];
        buf.push_resampled_16k(&chunk);
        buf.push_resampled_16k(&chunk);
        buf.push_resampled_16k(&chunk);
        assert_eq!(buf.samples.len(), WAKE_PREROLL_MAX_SAMPLES);
    }

    #[test]
    fn take_snapshot_clears_buffer() {
        let store = WakePreroll::default();
        store.push_resampled_16k(&[0.5, -0.25]);
        let snap = store.take_snapshot();
        assert_eq!(snap, vec![0.5, -0.25]);
        assert!(store.take_snapshot().is_empty());
    }
}
