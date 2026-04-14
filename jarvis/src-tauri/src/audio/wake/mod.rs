//! Wake-word detection (`WakeDetector` trait). Porcupine is the default Windows path (feature `porcupine`).

#[cfg(feature = "porcupine")]
pub mod porcupine;

#[cfg(feature = "oww")]
pub mod oww;

use thiserror::Error;

/// Picovoice service name for the Porcupine access key (same pattern as Anthropic in T4-4).
pub const KEYRING_SERVICE_PORCUPINE: &str = "jarvis-porcupine";
/// Credential entry label for the access key string.
pub const KEYRING_PORCUPINE_ACCESS_KEY: &str = "access_key";

/// Feed PCM chunks (16 kHz, mono, i16). Implementations validate frame length per engine.
pub trait WakeDetector: Send + 'static {
    /// Feed one chunk of PCM (16 kHz, i16, mono). Returns `true` when the wake phrase is detected.
    fn process_frame(&mut self, pcm: &[i16]) -> Result<bool, WakeError>;
    fn backend_name(&self) -> &'static str;
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WakeError {
    #[error("invalid pcm frame length: expected {expected}, got {actual}")]
    BadFrameLength { expected: usize, actual: usize },
    #[error("wake engine init failed: {0}")]
    Init(String),
    #[error("wake engine process failed: {0}")]
    Process(String),
    #[error("wake engine library: {0}")]
    Library(String),
}

pub(crate) fn expect_pcm_frame_len(actual: usize, expected: usize) -> Result<(), WakeError> {
    if actual != expected {
        Err(WakeError::BadFrameLength { expected, actual })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockWakeDetector {
        frame_len: usize,
        remaining_until_fire: usize,
    }

    impl MockWakeDetector {
        fn new(frame_len: usize, fire_after_frames: usize) -> Self {
            Self {
                frame_len,
                remaining_until_fire: fire_after_frames,
            }
        }
    }

    impl WakeDetector for MockWakeDetector {
        fn process_frame(&mut self, pcm: &[i16]) -> Result<bool, WakeError> {
            expect_pcm_frame_len(pcm.len(), self.frame_len)?;
            if self.remaining_until_fire > 0 {
                self.remaining_until_fire -= 1;
                Ok(false)
            } else {
                Ok(true)
            }
        }

        fn backend_name(&self) -> &'static str {
            "mock"
        }
    }

    #[test]
    fn expect_pcm_frame_len_rejects_mismatch() {
        let err = expect_pcm_frame_len(3, 512).unwrap_err();
        assert!(matches!(
            err,
            WakeError::BadFrameLength {
                expected: 512,
                actual: 3
            }
        ));
    }

    #[test]
    fn mock_wake_detector_returns_true_after_n_frames() {
        let mut d = MockWakeDetector::new(4, 1);
        assert_eq!(d.process_frame(&[0i16; 4]), Ok(false));
        assert_eq!(d.process_frame(&[0i16; 4]), Ok(true));
    }

    #[test]
    fn mock_wake_detector_rejects_wrong_length() {
        let mut d = MockWakeDetector::new(4, 0);
        assert!(d.process_frame(&[0i16; 3]).is_err());
    }
}
