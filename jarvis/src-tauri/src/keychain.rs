//! OS credential storage for API keys (never log secret values).

use crate::audio::wake::{KEYRING_PORCUPINE_ACCESS_KEY, KEYRING_SERVICE_PORCUPINE};
use keyring::Entry;

pub fn save_api_key(service: &str, key: &str) -> Result<(), String> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err("API key is empty".to_string());
    }
    let entry = match service {
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
