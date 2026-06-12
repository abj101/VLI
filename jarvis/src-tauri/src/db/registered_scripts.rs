//! Allowlisted user scripts for LLM `run_registered_script` tool (path + content hash).

#![allow(dead_code)]

use super::DbError;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisteredScript {
    pub id: String,
    pub path: String,
    pub hash: String,
    pub display_name: String,
    #[serde(default)]
    pub arg_names: Vec<String>,
}

pub fn ensure_registered_scripts_schema(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(
        r"
        CREATE TABLE IF NOT EXISTS registered_scripts (
            id TEXT PRIMARY KEY,
            path TEXT NOT NULL,
            hash TEXT NOT NULL,
            display_name TEXT NOT NULL,
            arg_names TEXT NOT NULL DEFAULT '[]',
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        ",
    )?;
    Ok(())
}

pub fn get_all_registered_scripts(conn: &Connection) -> Result<Vec<RegisteredScript>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, path, hash, display_name, arg_names FROM registered_scripts ORDER BY display_name ASC",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(row_to_script(row)?);
    }
    Ok(out)
}

pub fn get_registered_script(
    conn: &Connection,
    id: &str,
) -> Result<Option<RegisteredScript>, DbError> {
    let key = id.trim();
    if key.is_empty() {
        return Ok(None);
    }
    let mut stmt = conn.prepare(
        "SELECT id, path, hash, display_name, arg_names FROM registered_scripts WHERE id = ?1",
    )?;
    let mut rows = stmt.query(rusqlite::params![key])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row_to_script(row)?))
    } else {
        Ok(None)
    }
}

pub fn upsert_registered_script(
    conn: &Connection,
    script: &RegisteredScript,
) -> Result<(), DbError> {
    validate_script(script)?;
    let arg_names = serde_json::to_string(&script.arg_names)?;
    conn.execute(
        "INSERT INTO registered_scripts (id, path, hash, display_name, arg_names) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET path = excluded.path, hash = excluded.hash,
             display_name = excluded.display_name, arg_names = excluded.arg_names",
        rusqlite::params![
            script.id.trim(),
            script.path.trim(),
            script.hash.trim(),
            script.display_name.trim(),
            arg_names,
        ],
    )?;
    Ok(())
}

pub fn delete_registered_script(conn: &Connection, id: &str) -> Result<bool, DbError> {
    let n = conn.execute(
        "DELETE FROM registered_scripts WHERE id = ?1",
        [id.trim()],
    )?;
    Ok(n > 0)
}

/// SHA-256 hex digest of file bytes (used at register + run time).
pub fn hash_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read script `{path:?}`: {e}"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub fn verify_registered_script_file(script: &RegisteredScript) -> Result<(), String> {
    let path = Path::new(script.path.trim());
    if !path.is_file() {
        return Err(format!("registered script file not found: `{}`", script.path));
    }
    let current = hash_file(path)?;
    if current != script.hash {
        return Err(format!(
            "script `{}` hash mismatch (file changed since registration)",
            script.id
        ));
    }
    Ok(())
}

fn row_to_script(row: &rusqlite::Row<'_>) -> Result<RegisteredScript, DbError> {
    let arg_names: String = row.get(4)?;
    Ok(RegisteredScript {
        id: row.get(0)?,
        path: row.get(1)?,
        hash: row.get(2)?,
        display_name: row.get(3)?,
        arg_names: serde_json::from_str(&arg_names).unwrap_or_default(),
    })
}

fn validate_script(script: &RegisteredScript) -> Result<(), DbError> {
    if script.id.trim().is_empty() {
        return Err(DbError::Validation("script id is required".into()));
    }
    if !script
        .id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(DbError::Validation(
            "script id must be alphanumeric with underscores or hyphens".into(),
        ));
    }
    if script.path.trim().is_empty() {
        return Err(DbError::Validation("script path is required".into()));
    }
    if script.hash.trim().is_empty() {
        return Err(DbError::Validation("script hash is required".into()));
    }
    if script.display_name.trim().is_empty() {
        return Err(DbError::Validation("script display_name is required".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;
    use rusqlite::Connection;
    use std::io::Write;
    use tempfile::tempdir;

    fn open_temp() -> (tempfile::TempDir, Connection) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("scripts.db");
        init_db(&path).unwrap();
        (dir, Connection::open(&path).unwrap())
    }

    #[test]
    fn registered_script_crud_round_trip() {
        let (_dir, conn) = open_temp();
        upsert_registered_script(
            &conn,
            &RegisteredScript {
                id: "deploy".into(),
                path: r"C:\scripts\deploy.ps1".into(),
                hash: "abc123".into(),
                display_name: "Deploy staging".into(),
                arg_names: vec!["env".into()],
            },
        )
        .unwrap();

        let one = get_registered_script(&conn, "deploy").unwrap().expect("row");
        assert_eq!(one.display_name, "Deploy staging");
        assert_eq!(one.arg_names, vec!["env".to_string()]);

        assert!(delete_registered_script(&conn, "deploy").unwrap());
        assert!(get_registered_script(&conn, "deploy").unwrap().is_none());
    }

    #[test]
    fn hash_file_matches_content() {
        let dir = tempdir().unwrap();
        let script = dir.path().join("hello.cmd");
        let mut f = std::fs::File::create(&script).unwrap();
        writeln!(f, "@echo hello").unwrap();
        let h1 = hash_file(&script).unwrap();
        let h2 = hash_file(&script).unwrap();
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
    }
}
