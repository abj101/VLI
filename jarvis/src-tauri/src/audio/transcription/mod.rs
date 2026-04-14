//! Speech-to-text provider selection (Phase 4 T4-3). No LLM — audio → text only.
//!
//! ## Providers
//! - **`local`**: bundled Whisper (default; same behavior as Phase 1–3 when selected).
//! - **`os`**: platform speech APIs (Windows-first; other OSes may stay stubbed until implemented).
//! - **`remote`**: user BYOK HTTP endpoint; API key in OS keychain, never SQLite.
//!
//! ## Remote HTTP contract (BYOK)
//! The app sends `POST` to the configured HTTPS URL with JSON body:
//! ```json
//! {
//!   "model": "<optional, from settings>",
//!   "audio": {
//!     "encoding": "pcm_s16le",
//!     "sample_rate_hz": 16000,
//!     "channels": 1,
//!     "data": "<base64 raw PCM little-endian s16>"
//!   }
//! }
//! ```
//! Successful response must be JSON with a transcript string in **`text`** or **`transcript`**
//! (nested `{"result":{"text":"..."}}` is also accepted). Errors are surfaced via `audio-error`
//! without logging secret material.

mod os;
mod remote;

pub use os::spawn_os_stt_thread;
pub use remote::{spawn_remote_stt_thread, RemoteSttParams};

/// Stored in settings as lowercase ASCII.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SttProvider {
    Local,
    Os,
    Remote,
}

const STR_LOCAL: &str = "local";
const STR_OS: &str = "os";
const STR_REMOTE: &str = "remote";

/// Default matches historical behavior (Whisper-only pipeline).
pub fn parse_stt_provider(raw: Option<&str>) -> SttProvider {
    let s = raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_else(|| STR_LOCAL.to_string());
    match s.as_str() {
        STR_LOCAL => SttProvider::Local,
        STR_OS => SttProvider::Os,
        STR_REMOTE => SttProvider::Remote,
        _ => SttProvider::Local,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_defaults_to_local() {
        assert_eq!(parse_stt_provider(None), SttProvider::Local);
        assert_eq!(parse_stt_provider(Some("")), SttProvider::Local);
        assert_eq!(parse_stt_provider(Some("  ")), SttProvider::Local);
    }

    #[test]
    fn parse_accepts_variants() {
        assert_eq!(parse_stt_provider(Some("local")), SttProvider::Local);
        assert_eq!(parse_stt_provider(Some("OS")), SttProvider::Os);
        assert_eq!(parse_stt_provider(Some("Remote")), SttProvider::Remote);
    }

    #[test]
    fn parse_unknown_falls_back_to_local() {
        assert_eq!(parse_stt_provider(Some("haiku")), SttProvider::Local);
    }
}
