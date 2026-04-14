//! OS credential storage for API keys (never log secret values).

use crate::audio::wake::{KEYRING_PORCUPINE_ACCESS_KEY, KEYRING_SERVICE_PORCUPINE};
use keyring::Entry;

/// Remote STT BYOK API key (same pattern as Porcupine).
pub const KEYRING_SERVICE_REMOTE_STT: &str = "jarvis-remote-stt";
pub const KEYRING_REMOTE_STT_API_KEY: &str = "api_key";

pub fn save_api_key(service: &str, key: &str) -> Result<(), String> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err("API key is empty".to_string());
    }
    let entry = match service {
        "porcupine" => Entry::new(KEYRING_SERVICE_PORCUPINE, KEYRING_PORCUPINE_ACCESS_KEY)
            .map_err(|e| e.to_string())?,
        "remote_stt" => Entry::new(KEYRING_SERVICE_REMOTE_STT, KEYRING_REMOTE_STT_API_KEY)
            .map_err(|e| e.to_string())?,
        _ => return Err(format!("unknown API key service `{service}`")),
    };
    entry
        .set_password(trimmed)
        .map_err(|e| format!("failed to store key: {e}"))
}

pub fn delete_api_key(service: &str) -> Result<(), String> {
    let entry = match service {
        "porcupine" => Entry::new(KEYRING_SERVICE_PORCUPINE, KEYRING_PORCUPINE_ACCESS_KEY)
            .map_err(|e| e.to_string())?,
        "remote_stt" => Entry::new(KEYRING_SERVICE_REMOTE_STT, KEYRING_REMOTE_STT_API_KEY)
            .map_err(|e| e.to_string())?,
        _ => return Err(format!("unknown API key service `{service}`")),
    };
    match entry.delete_password() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("failed to delete key: {e}")),
    }
}

/// Returns stored secret for `service`, or `None` if missing.
pub fn get_api_key(service: &str) -> Result<Option<String>, String> {
    let entry = match service {
        "porcupine" => Entry::new(KEYRING_SERVICE_PORCUPINE, KEYRING_PORCUPINE_ACCESS_KEY)
            .map_err(|e| e.to_string())?,
        "remote_stt" => Entry::new(KEYRING_SERVICE_REMOTE_STT, KEYRING_REMOTE_STT_API_KEY)
            .map_err(|e| e.to_string())?,
        _ => return Err(format!("unknown API key service `{service}`")),
    };
    match entry.get_password() {
        Ok(p) => Ok(Some(p)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("failed to read key: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_service_rejected_without_echoing_key_material() {
        let secret = "unit-test-secret-never-log-this-xyz";
        let err = save_api_key("not_a_registered_service", secret).expect_err("unknown service");
        assert!(!err.contains(secret), "error must not echo key: {err}");
    }

    #[test]
    fn empty_key_rejected_without_echoing_value() {
        let err = save_api_key("porcupine", "   ").expect_err("empty key");
        assert_eq!(err, "API key is empty");
    }
}
