use crate::db::DbError;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

pub const SETTING_WAKE_ENGINE: &str = "wake_engine";
pub const SETTING_OWW_THRESHOLD: &str = "oww_threshold";
pub const DEFAULT_OWW_THRESHOLD: f32 = 0.7;
pub const SETTING_STT_PROVIDER: &str = "stt_provider";
pub const SETTING_REMOTE_STT_URL: &str = "remote_stt_url";
pub const SETTING_REMOTE_STT_MODEL: &str = "remote_stt_model";
pub const SETTING_REMOTE_STT_TIMEOUT_SECS: &str = "remote_stt_timeout_secs";
pub const SETTING_REMOTE_STT_KEY_STORED: &str = "remote_stt_key_stored";
pub const SETTING_LOCAL_WHISPER_USE_GPU: &str = "local_whisper_use_gpu";
pub const SETTING_LOCAL_WHISPER_MODEL: &str = "local_whisper_model";
pub const SETTING_LLM_ROUTER_MODEL_PATH: &str = "llm_router_model_path";
pub const SETTING_LLM_ROUTER_CONFIDENCE_THRESHOLD: &str = "llm_router_confidence_threshold";
pub const DEFAULT_LLM_ROUTER_CONFIDENCE_THRESHOLD: f32 = 0.7;
pub const SETTING_LLM_ROUTER_TIER2_ENABLED: &str = "llm_router_tier2_enabled";
pub const SETTING_LLM_ROUTER_WARMUP_ON_LAUNCH: &str = "llm_router_warmup_on_launch";
pub const SETTING_LLM_COMPOSER_ENABLED: &str = "llm_composer_enabled";
pub const SETTING_LLM_COMPOSER_MODEL_PATH: &str = "llm_composer_model_path";
pub const SETTING_LLM_COMPOSER_WARMUP_ON_LAUNCH: &str = "llm_composer_warmup_on_launch";
pub const SETTING_DICTATION_HOTKEY: &str = "dictation_hotkey";
pub const SETTING_DICTATION_HOTKEY_MODE: &str = "dictation_hotkey_mode";
pub const SETTING_HUD_COMMAND_POSITION: &str = "hud_command_position";
pub const SETTING_HUD_DICTATION_POSITION: &str = "hud_dictation_position";
pub const DEFAULT_DICTATION_HOTKEY: &str = "ctrl+shift+d";
pub const DEFAULT_DICTATION_HOTKEY_MODE: &str = "toggle";

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

/// Drops retired settings rows and normalizes values from removed features (best-effort idempotent).
pub fn prune_legacy_settings(conn: &Connection) -> Result<(), DbError> {
    conn.execute(
        "DELETE FROM settings WHERE key IN ('porcupine_key_stored')",
        [],
    )?;
    if let Some(raw) = get_setting(conn, SETTING_WAKE_ENGINE)? {
        if raw.trim().eq_ignore_ascii_case("porcupine") {
            set_setting(conn, SETTING_WAKE_ENGINE, "oww")?;
        }
    }
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
    match s.as_str() {
        "hotkey" => "hotkey".into(),
        "oww" => "oww".into(),
        "porcupine" => "oww".into(),
        _ => "oww".into(),
    }
}

fn parse_oww_threshold(raw: Option<String>) -> f32 {
    raw.and_then(|s| s.trim().parse::<f32>().ok())
        .filter(|t| t.is_finite() && *t > 0.0 && *t <= 1.0)
        .unwrap_or(DEFAULT_OWW_THRESHOLD)
}

fn parse_stt_provider_str(raw: Option<String>) -> String {
    use crate::audio::transcription::{parse_stt_provider, SttProvider};
    match parse_stt_provider(raw.as_deref()) {
        SttProvider::Local | SttProvider::Os => "local".to_string(),
        SttProvider::Remote => "remote".to_string(),
    }
}

fn parse_llm_router_confidence_threshold(raw: Option<String>) -> f32 {
    raw.and_then(|s| s.trim().parse::<f32>().ok())
        .filter(|t| t.is_finite() && *t >= 0.0 && *t <= 1.0)
        .unwrap_or(DEFAULT_LLM_ROUTER_CONFIDENCE_THRESHOLD)
}

fn parse_llm_composer_enabled(raw: Option<String>) -> bool {
    raw.map(|v| v.trim() == "1")
        .unwrap_or(cfg!(feature = "llm-local"))
}

fn parse_remote_stt_timeout_secs(raw: Option<String>) -> u32 {
    raw.and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|&n| n > 0 && n <= 300)
        .unwrap_or(30)
}

fn parse_local_whisper_model_str(raw: Option<String>) -> String {
    crate::audio::stt::parse_local_whisper_model_id(raw.as_deref()).to_string()
}

fn normalize_optional_trimmed(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

pub fn parse_dictation_hotkey_mode(raw: Option<&str>) -> String {
    match raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("push_to_talk") | Some("push-to-talk") | Some("ptt") => {
            "push_to_talk".to_string()
        }
        _ => DEFAULT_DICTATION_HOTKEY_MODE.to_string(),
    }
}

/// Serializable app settings for IPC — never includes secret key material.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub wake_engine: String,
    pub oww_threshold: f32,
    /// `local` | `os` | `remote`
    pub stt_provider: String,
    /// Non-secret remote endpoint base URL (HTTPS recommended).
    pub remote_stt_url: String,
    pub remote_stt_model: Option<String>,
    pub remote_stt_timeout_secs: u32,
    pub remote_stt_key_stored: bool,
    /// When true and the binary includes a GPU backend, local Whisper uses GPU (see `whisper_gpu_compile_supported`).
    pub local_whisper_use_gpu: bool,
    /// On-device Whisper model id: `tiny.en` | `base.en` | `small.en`.
    pub local_whisper_model: String,
    /// Optional override path to the router GGUF model.
    pub llm_router_model_path: Option<String>,
    /// Reject LLM router output below this confidence (0–1).
    pub llm_router_confidence_threshold: f32,
    /// When true, Tier 2 on-device LLM routing is allowed (Phase C).
    pub llm_router_tier2_enabled: bool,
    /// When true, warm the router model at app startup (if Tier 2 is enabled).
    pub llm_router_warmup_on_launch: bool,
    /// When true, the command composer (editor NL → draft) is allowed.
    pub llm_composer_enabled: bool,
    /// Optional override path to the composer GGUF model.
    pub llm_composer_model_path: Option<String>,
    /// When true, warm the composer model at app startup.
    pub llm_composer_warmup_on_launch: bool,
    /// Global shortcut to start/stop dictation into the focused text field.
    pub dictation_hotkey: String,
    /// `toggle` or `push_to_talk`.
    pub dictation_hotkey_mode: String,
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
    pub local_whisper_use_gpu: Option<bool>,
    pub local_whisper_model: Option<String>,
    pub llm_router_model_path: Option<String>,
    pub llm_router_confidence_threshold: Option<f32>,
    pub llm_router_tier2_enabled: Option<bool>,
    pub llm_router_warmup_on_launch: Option<bool>,
    pub llm_composer_enabled: Option<bool>,
    pub llm_composer_model_path: Option<String>,
    pub llm_composer_warmup_on_launch: Option<bool>,
    pub dictation_hotkey: Option<String>,
    pub dictation_hotkey_mode: Option<String>,
}

pub fn get_app_settings(conn: &Connection) -> Result<AppSettings, DbError> {
    Ok(AppSettings {
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
        local_whisper_use_gpu: bool_from_setting(get_setting(
            conn,
            SETTING_LOCAL_WHISPER_USE_GPU,
        )?),
        local_whisper_model: parse_local_whisper_model_str(get_setting(
            conn,
            SETTING_LOCAL_WHISPER_MODEL,
        )?),
        llm_router_model_path: normalize_optional_trimmed(get_setting(
            conn,
            SETTING_LLM_ROUTER_MODEL_PATH,
        )?),
        llm_router_confidence_threshold: parse_llm_router_confidence_threshold(get_setting(
            conn,
            SETTING_LLM_ROUTER_CONFIDENCE_THRESHOLD,
        )?),
        llm_router_tier2_enabled: bool_from_setting(get_setting(
            conn,
            SETTING_LLM_ROUTER_TIER2_ENABLED,
        )?),
        llm_router_warmup_on_launch: bool_from_setting(get_setting(
            conn,
            SETTING_LLM_ROUTER_WARMUP_ON_LAUNCH,
        )?),
        llm_composer_enabled: parse_llm_composer_enabled(get_setting(
            conn,
            SETTING_LLM_COMPOSER_ENABLED,
        )?),
        llm_composer_model_path: normalize_optional_trimmed(get_setting(
            conn,
            SETTING_LLM_COMPOSER_MODEL_PATH,
        )?),
        llm_composer_warmup_on_launch: bool_from_setting(get_setting(
            conn,
            SETTING_LLM_COMPOSER_WARMUP_ON_LAUNCH,
        )?),
        dictation_hotkey: get_setting(conn, SETTING_DICTATION_HOTKEY)?
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| DEFAULT_DICTATION_HOTKEY.to_string()),
        dictation_hotkey_mode: parse_dictation_hotkey_mode(
            get_setting(conn, SETTING_DICTATION_HOTKEY_MODE)?
                .as_deref(),
        ),
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
        if !matches!(normalized.as_str(), "hotkey" | "oww") {
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
        if normalized == "os" {
            return Err(DbError::Validation(
                "OS speech recognition is not available yet; use Local or Remote.".into(),
            ));
        }
        if !matches!(normalized.as_str(), "local" | "remote") {
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
    if let Some(on) = patch.local_whisper_use_gpu {
        set_setting(
            conn,
            SETTING_LOCAL_WHISPER_USE_GPU,
            if on { "1" } else { "0" },
        )?;
    }
    if let Some(ref raw) = patch.local_whisper_model {
        let normalized = raw.trim();
        if !crate::audio::stt::LOCAL_WHISPER_MODEL_IDS
            .iter()
            .any(|id| *id == normalized)
        {
            return Err(DbError::Validation(format!(
                "invalid local_whisper_model `{normalized}`"
            )));
        }
        set_setting(conn, SETTING_LOCAL_WHISPER_MODEL, normalized)?;
    }
    if let Some(ref path) = patch.llm_router_model_path {
        let stored = path.trim();
        if stored.is_empty() {
            set_setting(conn, SETTING_LLM_ROUTER_MODEL_PATH, "")?;
        } else {
            set_setting(conn, SETTING_LLM_ROUTER_MODEL_PATH, stored)?;
        }
    }
    if let Some(t) = patch.llm_router_confidence_threshold {
        if !(t.is_finite() && (0.0..=1.0).contains(&t)) {
            return Err(DbError::Validation(
                "llm_router_confidence_threshold must be between 0 and 1".into(),
            ));
        }
        set_setting(
            conn,
            SETTING_LLM_ROUTER_CONFIDENCE_THRESHOLD,
            &format!("{t}"),
        )?;
    }
    if let Some(on) = patch.llm_router_tier2_enabled {
        set_setting(
            conn,
            SETTING_LLM_ROUTER_TIER2_ENABLED,
            if on { "1" } else { "0" },
        )?;
    }
    if let Some(on) = patch.llm_router_warmup_on_launch {
        set_setting(
            conn,
            SETTING_LLM_ROUTER_WARMUP_ON_LAUNCH,
            if on { "1" } else { "0" },
        )?;
    }
    if let Some(on) = patch.llm_composer_enabled {
        set_setting(
            conn,
            SETTING_LLM_COMPOSER_ENABLED,
            if on { "1" } else { "0" },
        )?;
    }
    if let Some(ref path) = patch.llm_composer_model_path {
        let stored = path.trim();
        if stored.is_empty() {
            set_setting(conn, SETTING_LLM_COMPOSER_MODEL_PATH, "")?;
        } else {
            set_setting(conn, SETTING_LLM_COMPOSER_MODEL_PATH, stored)?;
        }
    }
    if let Some(on) = patch.llm_composer_warmup_on_launch {
        set_setting(
            conn,
            SETTING_LLM_COMPOSER_WARMUP_ON_LAUNCH,
            if on { "1" } else { "0" },
        )?;
    }
    if let Some(ref hotkey) = patch.dictation_hotkey {
        let trimmed = hotkey.trim();
        if trimmed.is_empty() {
            return Err(DbError::Validation("dictation_hotkey cannot be empty".into()));
        }
        set_setting(conn, SETTING_DICTATION_HOTKEY, trimmed)?;
    }
    if let Some(ref mode) = patch.dictation_hotkey_mode {
        let normalized = parse_dictation_hotkey_mode(Some(mode.as_str()));
        set_setting(conn, SETTING_DICTATION_HOTKEY_MODE, &normalized)?;
    }
    Ok(())
}

pub fn set_key_stored_flag(conn: &Connection, service: &str, stored: bool) -> Result<(), DbError> {
    let key = match service {
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
    fn prune_legacy_settings_clears_porcupine_and_migrates_wake_engine() {
        let (_dir, conn) = open_temp();
        set_setting(&conn, "porcupine_key_stored", "1").expect("seed legacy flag");
        set_setting(&conn, SETTING_WAKE_ENGINE, "porcupine").expect("seed legacy engine");
        prune_legacy_settings(&conn).expect("prune");
        assert_eq!(get_setting(&conn, "porcupine_key_stored").expect("get"), None);
        assert_eq!(
            get_setting(&conn, SETTING_WAKE_ENGINE).expect("get").as_deref(),
            Some("oww")
        );
    }

    #[test]
    fn get_app_settings_defaults_when_empty() {
        let (_dir, conn) = open_temp();
        let s = get_app_settings(&conn).expect("get_app_settings");
        assert_eq!(s.wake_engine, "oww");
        assert!((s.oww_threshold - DEFAULT_OWW_THRESHOLD).abs() < f32::EPSILON);
        assert_eq!(s.stt_provider, "local");
        assert_eq!(s.remote_stt_url, "");
        assert_eq!(s.remote_stt_model, None);
        assert_eq!(s.remote_stt_timeout_secs, 30);
        assert!(!s.remote_stt_key_stored);
        assert!(!s.local_whisper_use_gpu);
        assert_eq!(s.local_whisper_model, "base.en");
        assert_eq!(s.llm_router_model_path, None);
        assert!(
            (s.llm_router_confidence_threshold - DEFAULT_LLM_ROUTER_CONFIDENCE_THRESHOLD).abs()
                < f32::EPSILON
        );
        assert!(!s.llm_router_tier2_enabled);
        assert!(!s.llm_router_warmup_on_launch);
        assert_eq!(s.llm_composer_enabled, cfg!(feature = "llm-local"));
        assert_eq!(s.llm_composer_model_path, None);
        assert!(!s.llm_composer_warmup_on_launch);
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
                local_whisper_use_gpu: None,
                local_whisper_model: None,
                llm_router_model_path: None,
                llm_router_confidence_threshold: None,
                llm_router_tier2_enabled: None,
                llm_router_warmup_on_launch: None,
                llm_composer_enabled: None,
                llm_composer_model_path: None,
                llm_composer_warmup_on_launch: None,
                dictation_hotkey: None,
                dictation_hotkey_mode: None,
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
                local_whisper_use_gpu: None,
                local_whisper_model: None,
                llm_router_model_path: None,
                llm_router_confidence_threshold: None,
                llm_router_tier2_enabled: None,
                llm_router_warmup_on_launch: None,
                llm_composer_enabled: None,
                llm_composer_model_path: None,
                llm_composer_warmup_on_launch: None,
                dictation_hotkey: None,
                dictation_hotkey_mode: None,
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
                local_whisper_use_gpu: None,
                local_whisper_model: None,
                llm_router_model_path: None,
                llm_router_confidence_threshold: None,
                llm_router_tier2_enabled: None,
                llm_router_warmup_on_launch: None,
                llm_composer_enabled: None,
                llm_composer_model_path: None,
                llm_composer_warmup_on_launch: None,
                dictation_hotkey: None,
                dictation_hotkey_mode: None,
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
                local_whisper_use_gpu: None,
                local_whisper_model: None,
                llm_router_model_path: None,
                llm_router_confidence_threshold: None,
                llm_router_tier2_enabled: None,
                llm_router_warmup_on_launch: None,
                llm_composer_enabled: None,
                llm_composer_model_path: None,
                llm_composer_warmup_on_launch: None,
                dictation_hotkey: None,
                dictation_hotkey_mode: None,
            },
        )
        .expect_err("expected validation error");
        assert!(matches!(err, DbError::Validation(_)));
    }

    #[test]
    fn apply_settings_patch_persists_local_whisper_model() {
        let (_dir, conn) = open_temp();
        apply_settings_patch(
            &conn,
            &SettingsPatch {
                wake_engine: None,
                oww_threshold: None,
                stt_provider: None,
                remote_stt_url: None,
                remote_stt_model: None,
                remote_stt_timeout_secs: None,
                local_whisper_use_gpu: None,
                local_whisper_model: Some("base.en".into()),
                llm_router_model_path: None,
                llm_router_confidence_threshold: None,
                llm_router_tier2_enabled: None,
                llm_router_warmup_on_launch: None,
                llm_composer_enabled: None,
                llm_composer_model_path: None,
                llm_composer_warmup_on_launch: None,
                dictation_hotkey: None,
                dictation_hotkey_mode: None,
            },
        )
        .expect("patch");
        let s = get_app_settings(&conn).expect("reload");
        assert_eq!(s.local_whisper_model, "base.en");
    }

    #[test]
    fn invalid_local_whisper_model_rejected() {
        let (_dir, conn) = open_temp();
        let err = apply_settings_patch(
            &conn,
            &SettingsPatch {
                wake_engine: None,
                oww_threshold: None,
                stt_provider: None,
                remote_stt_url: None,
                remote_stt_model: None,
                remote_stt_timeout_secs: None,
                local_whisper_use_gpu: None,
                local_whisper_model: Some("large-v3".into()),
                llm_router_model_path: None,
                llm_router_confidence_threshold: None,
                llm_router_tier2_enabled: None,
                llm_router_warmup_on_launch: None,
                llm_composer_enabled: None,
                llm_composer_model_path: None,
                llm_composer_warmup_on_launch: None,
                dictation_hotkey: None,
                dictation_hotkey_mode: None,
            },
        )
        .expect_err("expected validation error");
        assert!(matches!(err, DbError::Validation(_)));
    }

    #[test]
    fn apply_settings_patch_persists_local_whisper_use_gpu() {
        let (_dir, conn) = open_temp();
        apply_settings_patch(
            &conn,
            &SettingsPatch {
                wake_engine: None,
                oww_threshold: None,
                stt_provider: None,
                remote_stt_url: None,
                remote_stt_model: None,
                remote_stt_timeout_secs: None,
                local_whisper_use_gpu: Some(true),
                local_whisper_model: None,
                llm_router_model_path: None,
                llm_router_confidence_threshold: None,
                llm_router_tier2_enabled: None,
                llm_router_warmup_on_launch: None,
                llm_composer_enabled: None,
                llm_composer_model_path: None,
                llm_composer_warmup_on_launch: None,
                dictation_hotkey: None,
                dictation_hotkey_mode: None,
            },
        )
        .expect("patch");
        let s = get_app_settings(&conn).expect("reload");
        assert!(s.local_whisper_use_gpu);
        apply_settings_patch(
            &conn,
            &SettingsPatch {
                wake_engine: None,
                oww_threshold: None,
                stt_provider: None,
                remote_stt_url: None,
                remote_stt_model: None,
                remote_stt_timeout_secs: None,
                local_whisper_use_gpu: Some(false),
                local_whisper_model: None,
                llm_router_model_path: None,
                llm_router_confidence_threshold: None,
                llm_router_tier2_enabled: None,
                llm_router_warmup_on_launch: None,
                llm_composer_enabled: None,
                llm_composer_model_path: None,
                llm_composer_warmup_on_launch: None,
                dictation_hotkey: None,
                dictation_hotkey_mode: None,
            },
        )
        .expect("patch off");
        let s = get_app_settings(&conn).expect("reload");
        assert!(!s.local_whisper_use_gpu);
        assert_eq!(s.local_whisper_model, "base.en");
    }

    #[test]
    fn apply_settings_patch_persists_llm_router_settings() {
        let (_dir, conn) = open_temp();
        apply_settings_patch(
            &conn,
            &SettingsPatch {
                wake_engine: None,
                oww_threshold: None,
                stt_provider: None,
                remote_stt_url: None,
                remote_stt_model: None,
                remote_stt_timeout_secs: None,
                local_whisper_use_gpu: None,
                local_whisper_model: None,
                llm_router_model_path: Some(r"C:\models\router.gguf".into()),
                llm_router_confidence_threshold: Some(0.55),
                llm_router_tier2_enabled: Some(true),
                llm_router_warmup_on_launch: Some(true),
                llm_composer_enabled: None,
                llm_composer_model_path: None,
                llm_composer_warmup_on_launch: None,
                dictation_hotkey: None,
                dictation_hotkey_mode: None,
            },
        )
        .expect("patch");
        let s = get_app_settings(&conn).expect("reload");
        assert_eq!(
            s.llm_router_model_path.as_deref(),
            Some(r"C:\models\router.gguf")
        );
        assert!((s.llm_router_confidence_threshold - 0.55).abs() < 0.0001);
        assert!(s.llm_router_tier2_enabled);
        assert!(s.llm_router_warmup_on_launch);
    }

    #[test]
    fn dictation_settings_default_to_ctrl_shift_d_toggle() {
        let (_dir, conn) = open_temp();
        let s = get_app_settings(&conn).expect("settings");
        assert_eq!(s.dictation_hotkey, "ctrl+shift+d");
        assert_eq!(s.dictation_hotkey_mode, "toggle");
    }

    #[test]
    fn parse_dictation_hotkey_mode_normalizes_ptt() {
        assert_eq!(
            parse_dictation_hotkey_mode(Some("push-to-talk")),
            "push_to_talk"
        );
        assert_eq!(parse_dictation_hotkey_mode(Some("ptt")), "push_to_talk");
        assert_eq!(parse_dictation_hotkey_mode(Some("toggle")), "toggle");
        assert_eq!(parse_dictation_hotkey_mode(None), "toggle");
    }
}
