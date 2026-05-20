//! OS credential storage for API keys (never log secret values).

use keyring::Entry;

/// Remote STT BYOK API key.
pub const KEYRING_SERVICE_REMOTE_STT: &str = "jarvis-remote-stt";
pub const KEYRING_REMOTE_STT_API_KEY: &str = "api_key";

/// Removes credentials from retired integrations (best-effort; ignores missing entries).
pub fn purge_retired_credentials() {
    const LEGACY_PORCUPINE_SERVICE: &str = "jarvis-porcupine";
    const LEGACY_PORCUPINE_USER: &str = "access_key";
    if let Ok(entry) = Entry::new(LEGACY_PORCUPINE_SERVICE, LEGACY_PORCUPINE_USER) {
        let _ = entry.delete_password();
    }
}

pub fn save_api_key(service: &str, key: &str) -> Result<(), String> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err("API key is empty".to_string());
    }
    let entry = match service {
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
        let err = save_api_key("remote_stt", "   ").expect_err("empty key");
        assert_eq!(err, "API key is empty");
    }
}
