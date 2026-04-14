use crate::db::DbError;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

pub const SETTING_ANTHROPIC_KEY_STORED: &str = "anthropic_key_stored";
pub const SETTING_PORCUPINE_KEY_STORED: &str = "porcupine_key_stored";
pub const SETTING_WAKE_ENGINE: &str = "wake_engine";
pub const SETTING_OWW_THRESHOLD: &str = "oww_threshold";
pub const SETTING_GLOBAL_AI_MODE: &str = "global_ai_mode";

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
        .unwrap_or_else(|| "hotkey".to_string());
    if matches!(s.as_str(), "hotkey" | "porcupine" | "oww") {
        s
    } else {
        "hotkey".to_string()
    }
}

fn parse_oww_threshold(raw: Option<String>) -> f32 {
    raw.and_then(|s| s.trim().parse::<f32>().ok())
        .filter(|t| t.is_finite() && *t > 0.0 && *t <= 1.0)
        .unwrap_or(0.5)
}

/// Serializable app settings for IPC — never includes secret key material.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub anthropic_key_stored: bool,
    pub porcupine_key_stored: bool,
    pub wake_engine: String,
    pub oww_threshold: f32,
    pub global_ai_mode: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub wake_engine: Option<String>,
    pub oww_threshold: Option<f32>,
    pub global_ai_mode: Option<bool>,
}

pub fn get_app_settings(conn: &Connection) -> Result<AppSettings, DbError> {
    Ok(AppSettings {
        anthropic_key_stored: bool_from_setting(get_setting(conn, SETTING_ANTHROPIC_KEY_STORED)?),
        porcupine_key_stored: bool_from_setting(get_setting(conn, SETTING_PORCUPINE_KEY_STORED)?),
        wake_engine: parse_wake_engine(get_setting(conn, SETTING_WAKE_ENGINE)?),
        oww_threshold: parse_oww_threshold(get_setting(conn, SETTING_OWW_THRESHOLD)?),
        global_ai_mode: bool_from_setting(get_setting(conn, SETTING_GLOBAL_AI_MODE)?),
    })
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
    if let Some(on) = patch.global_ai_mode {
        set_setting(conn, SETTING_GLOBAL_AI_MODE, if on { "1" } else { "0" })?;
    }
    Ok(())
}

pub fn set_key_stored_flag(conn: &Connection, service: &str, stored: bool) -> Result<(), DbError> {
    let key = match service {
        "anthropic" => SETTING_ANTHROPIC_KEY_STORED,
        "porcupine" => SETTING_PORCUPINE_KEY_STORED,
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
        assert!(!s.anthropic_key_stored);
        assert!(!s.porcupine_key_stored);
        assert_eq!(s.wake_engine, "hotkey");
        assert!((s.oww_threshold - 0.5).abs() < f32::EPSILON);
        assert!(!s.global_ai_mode);
    }

    #[test]
    fn apply_settings_patch_persists_wake_and_ai_mode() {
        let (_dir, conn) = open_temp();
        apply_settings_patch(
            &conn,
            &SettingsPatch {
                wake_engine: Some("oww".into()),
                oww_threshold: Some(0.35),
                global_ai_mode: Some(true),
            },
        )
        .expect("patch");
        let s = get_app_settings(&conn).expect("reload");
        assert_eq!(s.wake_engine, "oww");
        assert!((s.oww_threshold - 0.35).abs() < 0.0001);
        assert!(s.global_ai_mode);
    }

    #[test]
    fn invalid_wake_engine_rejected() {
        let (_dir, conn) = open_temp();
        let err = apply_settings_patch(
            &conn,
            &SettingsPatch {
                wake_engine: Some("bogus".into()),
                oww_threshold: None,
                global_ai_mode: None,
            },
        )
        .expect_err("expected validation error");
        assert!(
            matches!(err, DbError::Validation(_)),
            "expected Validation, got {err:?}"
        );
    }
}
