//! Spoken-name overrides for `open_target` resolution (`target_aliases` table).

use super::DbError;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetAliasKind {
    App,
    Url,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetAlias {
    pub spoken: String,
    pub kind: TargetAliasKind,
    pub value: String,
}

pub fn ensure_target_aliases_schema(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(
        r"
        CREATE TABLE IF NOT EXISTS target_aliases (
            spoken TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            value TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        ",
    )?;
    Ok(())
}

pub fn get_all_target_aliases(conn: &Connection) -> Result<Vec<TargetAlias>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT spoken, kind, value FROM target_aliases ORDER BY spoken ASC",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(row_to_alias(row)?);
    }
    Ok(out)
}

pub fn get_target_alias(conn: &Connection, spoken: &str) -> Result<Option<TargetAlias>, DbError> {
    let key = spoken.trim().to_lowercase();
    if key.is_empty() {
        return Ok(None);
    }
    let mut stmt =
        conn.prepare("SELECT spoken, kind, value FROM target_aliases WHERE spoken = ?1")?;
    let mut rows = stmt.query(rusqlite::params![key])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row_to_alias(row)?))
    } else {
        Ok(None)
    }
}

pub fn upsert_target_alias(conn: &Connection, alias: &TargetAlias) -> Result<(), DbError> {
    validate_alias(alias)?;
    let spoken = alias.spoken.trim().to_lowercase();
    let kind = kind_to_db(&alias.kind);
    conn.execute(
        "INSERT INTO target_aliases (spoken, kind, value) VALUES (?1, ?2, ?3)
         ON CONFLICT(spoken) DO UPDATE SET kind = excluded.kind, value = excluded.value",
        rusqlite::params![spoken, kind, alias.value.trim()],
    )?;
    Ok(())
}

pub fn delete_target_alias(conn: &Connection, spoken: &str) -> Result<bool, DbError> {
    let key = spoken.trim().to_lowercase();
    let n = conn.execute("DELETE FROM target_aliases WHERE spoken = ?1", [key])?;
    Ok(n > 0)
}

fn row_to_alias(row: &rusqlite::Row<'_>) -> Result<TargetAlias, DbError> {
    let kind_raw: String = row.get(1)?;
    Ok(TargetAlias {
        spoken: row.get(0)?,
        kind: kind_from_db(&kind_raw),
        value: row.get(2)?,
    })
}

fn kind_to_db(kind: &TargetAliasKind) -> &'static str {
    match kind {
        TargetAliasKind::App => "app",
        TargetAliasKind::Url => "url",
    }
}

fn kind_from_db(raw: &str) -> TargetAliasKind {
    match raw.trim().to_ascii_lowercase().as_str() {
        "app" => TargetAliasKind::App,
        _ => TargetAliasKind::Url,
    }
}

fn validate_alias(alias: &TargetAlias) -> Result<(), DbError> {
    if alias.spoken.trim().is_empty() {
        return Err(DbError::Validation("alias spoken name is required".into()));
    }
    if alias.value.trim().is_empty() {
        return Err(DbError::Validation("alias value is required".into()));
    }
    if matches!(alias.kind, TargetAliasKind::Url) {
        let v = alias.value.trim();
        if !v.starts_with("http://") && !v.starts_with("https://") {
            return Err(DbError::Validation(
                "url alias value must start with http:// or https://".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;
    use rusqlite::Connection;
    use tempfile::tempdir;

    fn open_temp() -> (tempfile::TempDir, Connection) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("aliases.db");
        init_db(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        (dir, conn)
    }

    #[test]
    fn target_alias_crud_round_trip() {
        let (_dir, conn) = open_temp();
        upsert_target_alias(
            &conn,
            &TargetAlias {
                spoken: "GitHub".into(),
                kind: TargetAliasKind::Url,
                value: "https://github.com".into(),
            },
        )
        .unwrap();

        let one = get_target_alias(&conn, "github").unwrap().expect("row");
        assert_eq!(one.spoken, "github");
        assert_eq!(one.kind, TargetAliasKind::Url);

        let all = get_all_target_aliases(&conn).unwrap();
        assert_eq!(all.len(), 1);

        assert!(delete_target_alias(&conn, "github").unwrap());
        assert!(get_all_target_aliases(&conn).unwrap().is_empty());
    }

    #[test]
    fn rejects_invalid_url_alias() {
        let (_dir, conn) = open_temp();
        let err = upsert_target_alias(
            &conn,
            &TargetAlias {
                spoken: "bad".into(),
                kind: TargetAliasKind::Url,
                value: "not-a-url".into(),
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("http"));
    }
}
