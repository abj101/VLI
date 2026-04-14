use crate::db::DbError;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

pub const SETTING_PORCUPINE_KEY_STORED: &str = "porcupine_key_stored";
pub const SETTING_WAKE_ENGINE: &str = "wake_engine";
pub const SETTING_OWW_THRESHOLD: &str = "oww_threshold";
pub const SETTING_STT_PROVIDER: &str = "stt_provider";
pub const SETTING_REMOTE_STT_URL: &str = "remote_stt_url";
pub const SETTING_REMOTE_STT_MODEL: &str = "remote_stt_model";
pub const SETTING_REMOTE_STT_TIMEOUT_SECS: &str = "remote_stt_timeout_secs";
pub const SETTING_REMOTE_STT_KEY_STORED: &str = "remote_stt_key_stored";

pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>, DbError> {
    let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
    let mut rows = stmt.query(rusqlite::params![key])?;
    if let Some(row) = rows.next()? {
        let value: String = row.get(0)?;
        Ok(Some(value))
    } else {
        Ok(None)
    }
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

fn bool_from_setting(raw: Option<String>) -> bool {
    raw.map(|v| v.trim() == "1").unwrap_or(false)
}

fn parse_wake_engine(raw: Option<String>) -> String {
    let s = raw
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "oww".to_string());
    if matches!(s.as_str(), "hotkey" | "porcupine" | "oww") {
        s
    } else {
        "oww".to_string()
    }
}

fn parse_oww_threshold(raw: Option<String>) -> f32 {
    raw.and_then(|s| s.trim().parse::<f32>().ok())
        .filter(|t| t.is_finite() && *t > 0.0 && *t <= 1.0)
        .unwrap_or(0.5)
}

fn parse_stt_provider_str(raw: Option<String>) -> String {
    let s = raw
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "local".to_string());
    if matches!(s.as_str(), "local" | "os" | "remote") {
        s
    } else {
        "local".to_string()
    }
}

fn parse_remote_stt_timeout_secs(raw: Option<String>) -> u32 {
    raw.and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|&n| n > 0 && n <= 300)
        .unwrap_or(30)
}

fn normalize_optional_trimmed(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// Serializable app settings for IPC — never includes secret key material.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub porcupine_key_stored: bool,
    pub wake_engine: String,
    pub oww_threshold: f32,
    /// `local` | `os` | `remote`
    pub stt_provider: String,
    /// Non-secret remote endpoint base URL (HTTPS recommended).
    pub remote_stt_url: String,
    pub remote_stt_model: Option<String>,
    pub remote_stt_timeout_secs: u32,
    pub remote_stt_key_stored: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub wake_engine: Option<String>,
    pub oww_threshold: Option<f32>,
    pub stt_provider: Option<String>,
    pub remote_stt_url: Option<String>,
    pub remote_stt_model: Option<String>,
    pub remote_stt_timeout_secs: Option<u32>,
}

pub fn get_app_settings(conn: &Connection) -> Result<AppSettings, DbError> {
    Ok(AppSettings {
        porcupine_key_stored: bool_from_setting(get_setting(conn, SETTING_PORCUPINE_KEY_STORED)?),
        wake_engine: parse_wake_engine(get_setting(conn, SETTING_WAKE_ENGINE)?),
        oww_threshold: parse_oww_threshold(get_setting(conn, SETTING_OWW_THRESHOLD)?),
        stt_provider: parse_stt_provider_str(get_setting(conn, SETTING_STT_PROVIDER)?),
        remote_stt_url: get_setting(conn, SETTING_REMOTE_STT_URL)?
            .map(|v| v.trim().to_string())
            .unwrap_or_default(),
        remote_stt_model: normalize_optional_trimmed(get_setting(conn, SETTING_REMOTE_STT_MODEL)?),
        remote_stt_timeout_secs: parse_remote_stt_timeout_secs(get_setting(
            conn,
            SETTING_REMOTE_STT_TIMEOUT_SECS,
        )?),
        remote_stt_key_stored: bool_from_setting(get_setting(conn, SETTING_REMOTE_STT_KEY_STORED)?),
    })
}

fn validate_remote_stt_url(url: &str) -> Result<(), DbError> {
    let t = url.trim();
    if t.is_empty() {
        return Ok(());
    }
    let parsed = url::Url::parse(t)
        .map_err(|e| DbError::Validation(format!("invalid remote_stt_url: {e}")))?;
    match parsed.scheme() {
        "http" | "https" => Ok(()),
        other => Err(DbError::Validation(format!(
            "remote_stt_url must use http or https, got `{other}`"
        ))),
    }
}

pub fn apply_settings_patch(conn: &Connection, patch: &SettingsPatch) -> Result<(), DbError> {
    if let Some(ref raw) = patch.wake_engine {
        let normalized = raw.trim().to_ascii_lowercase();
        if !matches!(normalized.as_str(), "hotkey" | "porcupine" | "oww") {
            return Err(DbError::Validation(format!(
                "invalid wake_engine `{normalized}`"
            )));
        }
        set_setting(conn, SETTING_WAKE_ENGINE, &normalized)?;
    }
    if let Some(t) = patch.oww_threshold {
        if !(t.is_finite() && t > 0.0 && t <= 1.0) {
            return Err(DbError::Validation(
                "oww_threshold must be greater than 0 and at most 1".into(),
            ));
        }
        set_setting(conn, SETTING_OWW_THRESHOLD, &format!("{t}"))?;
    }
    if let Some(ref raw) = patch.stt_provider {
        let normalized = raw.trim().to_ascii_lowercase();
        if !matches!(normalized.as_str(), "local" | "os" | "remote") {
            return Err(DbError::Validation(format!(
                "invalid stt_provider `{normalized}`"
            )));
        }
        set_setting(conn, SETTING_STT_PROVIDER, &normalized)?;
    }
    if let Some(ref raw) = patch.remote_stt_url {
        validate_remote_stt_url(raw)?;
        set_setting(conn, SETTING_REMOTE_STT_URL, raw.trim())?;
    }
    if let Some(ref model) = patch.remote_stt_model {
        let stored = model.trim();
        if stored.is_empty() {
            set_setting(conn, SETTING_REMOTE_STT_MODEL, "")?;
        } else {
            set_setting(conn, SETTING_REMOTE_STT_MODEL, stored)?;
        }
    }
    if let Some(secs) = patch.remote_stt_timeout_secs {
        if secs == 0 || secs > 300 {
            return Err(DbError::Validation(
                "remote_stt_timeout_secs must be between 1 and 300".into(),
            ));
        }
        set_setting(conn, SETTING_REMOTE_STT_TIMEOUT_SECS, &format!("{secs}"))?;
    }
    Ok(())
}

pub fn set_key_stored_flag(conn: &Connection, service: &str, stored: bool) -> Result<(), DbError> {
    let key = match service {
        "porcupine" => SETTING_PORCUPINE_KEY_STORED,
        "remote_stt" => SETTING_REMOTE_STT_KEY_STORED,
        _ => {
            return Err(DbError::Validation(format!(
                "unknown API key service `{service}`"
            )));
        }
    };
    set_setting(conn, key, if stored { "1" } else { "0" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;
    use rusqlite::Connection;
    use tempfile::tempdir;

    fn open_temp() -> (tempfile::TempDir, Connection) {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("settings-test.db");
        init_db(&path).expect("init db");
        let conn = Connection::open(&path).expect("open db");
        (dir, conn)
    }

    #[test]
    fn round_trip_insert_update_get_setting() {
        let (_dir, conn) = open_temp();
        assert_eq!(get_setting(&conn, "theme").expect("get before set"), None);

        set_setting(&conn, "theme", "dark").expect("set dark");
        assert_eq!(
            get_setting(&conn, "theme").expect("get dark"),
            Some("dark".to_string())
        );

        set_setting(&conn, "theme", "light").expect("set light");
        assert_eq!(
            get_setting(&conn, "theme").expect("get light"),
            Some("light".to_string())
        );
    }

    #[test]
    fn get_app_settings_defaults_when_empty() {
        let (_dir, conn) = open_temp();
        let s = get_app_settings(&conn).expect("get_app_settings");
        assert!(!s.porcupine_key_stored);
        assert_eq!(s.wake_engine, "oww");
        assert!((s.oww_threshold - 0.5).abs() < f32::EPSILON);
        assert_eq!(s.stt_provider, "local");
        assert_eq!(s.remote_stt_url, "");
        assert_eq!(s.remote_stt_model, None);
        assert_eq!(s.remote_stt_timeout_secs, 30);
        assert!(!s.remote_stt_key_stored);
    }

    #[test]
    fn apply_settings_patch_persists_wake_and_oww() {
        let (_dir, conn) = open_temp();
        apply_settings_patch(
            &conn,
            &SettingsPatch {
                wake_engine: Some("oww".into()),
                oww_threshold: Some(0.35),
                stt_provider: None,
                remote_stt_url: None,
                remote_stt_model: None,
                remote_stt_timeout_secs: None,
            },
        )
        .expect("patch");
        let s = get_app_settings(&conn).expect("reload");
        assert_eq!(s.wake_engine, "oww");
        assert!((s.oww_threshold - 0.35).abs() < 0.0001);
    }

    #[test]
    fn invalid_wake_engine_rejected() {
        let (_dir, conn) = open_temp();
        let err = apply_settings_patch(
            &conn,
            &SettingsPatch {
                wake_engine: Some("bogus".into()),
                oww_threshold: None,
                stt_provider: None,
                remote_stt_url: None,
                remote_stt_model: None,
                remote_stt_timeout_secs: None,
            },
        )
        .expect_err("expected validation error");
        assert!(
            matches!(err, DbError::Validation(_)),
            "expected Validation, got {err:?}"
        );
    }

    #[test]
    fn apply_settings_patch_stt_provider_and_remote_url() {
        let (_dir, conn) = open_temp();
        apply_settings_patch(
            &conn,
            &SettingsPatch {
                wake_engine: None,
                oww_threshold: None,
                stt_provider: Some("remote".into()),
                remote_stt_url: Some("https://example.com/v1/transcribe".into()),
                remote_stt_model: Some("test-model".into()),
                remote_stt_timeout_secs: Some(60),
            },
        )
        .expect("patch");
        let s = get_app_settings(&conn).expect("reload");
        assert_eq!(s.stt_provider, "remote");
        assert_eq!(s.remote_stt_url, "https://example.com/v1/transcribe");
        assert_eq!(s.remote_stt_model.as_deref(), Some("test-model"));
        assert_eq!(s.remote_stt_timeout_secs, 60);
    }

    #[test]
    fn invalid_remote_stt_url_rejected() {
        let (_dir, conn) = open_temp();
        let err = apply_settings_patch(
            &conn,
            &SettingsPatch {
                wake_engine: None,
                oww_threshold: None,
                stt_provider: None,
                remote_stt_url: Some("ftp://bad.example/transcribe".into()),
                remote_stt_model: None,
                remote_stt_timeout_secs: None,
            },
        )
        .expect_err("expected validation error");
        assert!(matches!(err, DbError::Validation(_)));
    }
}
