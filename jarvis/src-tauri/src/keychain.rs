//! OS credential storage for API keys (never log secret values).

use crate::audio::wake::{KEYRING_PORCUPINE_ACCESS_KEY, KEYRING_SERVICE_PORCUPINE};
use keyring::Entry;

const KEYRING_SERVICE_ANTHROPIC: &str = "jarvis-anthropic";
const KEYRING_ANTHROPIC_USERNAME: &str = "api_key";

/// Returns true if a non-empty Anthropic key exists in the keychain (does not read value into logs).
pub fn anthropic_key_in_keychain() -> bool {
    let Ok(entry) = Entry::new(KEYRING_SERVICE_ANTHROPIC, KEYRING_ANTHROPIC_USERNAME) else {
        return false;
    };
    entry
        .get_password()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
}

pub fn save_api_key(service: &str, key: &str) -> Result<(), String> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err("API key is empty".to_string());
    }
    let entry = match service {
        "anthropic" => Entry::new(KEYRING_SERVICE_ANTHROPIC, KEYRING_ANTHROPIC_USERNAME)
            .map_err(|e| e.to_string())?,
        "porcupine" => Entry::new(KEYRING_SERVICE_PORCUPINE, KEYRING_PORCUPINE_ACCESS_KEY)
            .map_err(|e| e.to_string())?,
        _ => return Err(format!("unknown API key service `{service}`")),
    };
    entry
        .set_password(trimmed)
        .map_err(|e| format!("failed to store key: {e}"))
}

pub fn delete_api_key(service: &str) -> Result<(), String> {
    let entry = match service {
        "anthropic" => Entry::new(KEYRING_SERVICE_ANTHROPIC, KEYRING_ANTHROPIC_USERNAME)
            .map_err(|e| e.to_string())?,
        "porcupine" => Entry::new(KEYRING_SERVICE_PORCUPINE, KEYRING_PORCUPINE_ACCESS_KEY)
            .map_err(|e| e.to_string())?,
        _ => return Err(format!("unknown API key service `{service}`")),
    };
    match entry.delete_password() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("failed to delete key: {e}")),
    }
}
