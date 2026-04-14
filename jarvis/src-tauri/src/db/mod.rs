//! SQLite command storage (`command_nodes`).

mod models;

pub use models::{Action, CommandNode, NewCommandNode};

use rusqlite::Connection;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
}

/// Open or create DB file, apply schema, seed defaults when table empty.
pub fn init_db(path: &Path) -> Result<(), DbError> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        r"
        PRAGMA foreign_keys = ON;
        CREATE TABLE IF NOT EXISTS command_nodes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            trigger_phrases TEXT NOT NULL,
            actions TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        ",
    )?;
    maybe_seed(&conn)?;
    Ok(())
}

fn maybe_seed(conn: &Connection) -> Result<(), DbError> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM command_nodes", [], |r| r.get(0))?;
    if count > 0 {
        return Ok(());
    }
    insert_command(
        conn,
        &NewCommandNode {
            name: "Open Notepad".into(),
            trigger_phrases: vec!["open notepad".into()],
            actions: vec![Action::OpenApp {
                name: "notepad".into(),
                path: "notepad.exe".into(),
            }],
            enabled: true,
        },
    )?;
    insert_command(
        conn,
        &NewCommandNode {
            name: "Open GitHub".into(),
            trigger_phrases: vec!["open github".into()],
            actions: vec![Action::OpenUrl {
                url: "https://github.com".into(),
            }],
            enabled: true,
        },
    )?;
    Ok(())
}

pub fn insert_command(conn: &Connection, row: &NewCommandNode) -> Result<i64, DbError> {
    let trigger_phrases = serde_json::to_string(&row.trigger_phrases)?;
    let actions = serde_json::to_string(&row.actions)?;
    let enabled = i32::from(row.enabled);
    conn.execute(
        "INSERT INTO command_nodes (name, trigger_phrases, actions, enabled) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![row.name, trigger_phrases, actions, enabled],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_all_commands(conn: &Connection) -> Result<Vec<CommandNode>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, trigger_phrases, actions, enabled, created_at FROM command_nodes ORDER BY id ASC",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(row_to_command(&row)?);
    }
    Ok(out)
}

pub fn get_command_by_id(conn: &Connection, id: i64) -> Result<Option<CommandNode>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, trigger_phrases, actions, enabled, created_at FROM command_nodes WHERE id = ?1",
    )?;
    let mut rows = stmt.query(rusqlite::params![id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row_to_command(&row)?))
    } else {
        Ok(None)
    }
}

pub fn delete_command(conn: &Connection, id: i64) -> Result<bool, DbError> {
    let n = conn.execute("DELETE FROM command_nodes WHERE id = ?1", [id])?;
    Ok(n > 0)
}

fn row_to_command(row: &rusqlite::Row<'_>) -> Result<CommandNode, DbError> {
    let trigger_phrases: String = row.get(2)?;
    let actions: String = row.get(3)?;
    let enabled_i: i32 = row.get(4)?;
    Ok(CommandNode {
        id: row.get(0)?,
        name: row.get(1)?,
        trigger_phrases: serde_json::from_str(&trigger_phrases)?,
        actions: serde_json::from_str(&actions)?,
        enabled: enabled_i != 0,
        created_at: row.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::tempdir;

    fn open_temp() -> (tempfile::TempDir, Connection) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        init_db(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        (dir, conn)
    }

    #[test]
    fn seed_runs_once_and_matches_expected_actions() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("j.db");
        init_db(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        let all = get_all_commands(&conn).unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|n| {
            n.trigger_phrases.contains(&"open notepad".to_string())
                && n.actions.iter().any(|a| {
                    matches!(
                        a,
                        Action::OpenApp { name, path }
                            if name == "notepad" && path == "notepad.exe"
                    )
                })
        }));
        assert!(all.iter().any(|n| {
            n.trigger_phrases.contains(&"open github".to_string())
                && n.actions.iter().any(|a| {
                    matches!(
                        a,
                        Action::OpenUrl { url } if url == "https://github.com"
                    )
                })
        }));

        init_db(&path).unwrap();
        let conn2 = Connection::open(&path).unwrap();
        assert_eq!(get_all_commands(&conn2).unwrap().len(), 2);
    }

    #[test]
    fn insert_list_get_delete() {
        let (_dir, conn) = open_temp();
        let id = insert_command(
            &conn,
            &NewCommandNode {
                name: "Custom".into(),
                trigger_phrases: vec!["do thing".into()],
                actions: vec![Action::OpenUrl {
                    url: "https://example.com".into(),
                }],
                enabled: false,
            },
        )
        .unwrap();
        let one = get_command_by_id(&conn, id).unwrap().expect("row");
        assert_eq!(one.name, "Custom");
        assert!(!one.enabled);
        assert_eq!(one.trigger_phrases, vec!["do thing".to_string()]);

        let all = get_all_commands(&conn).unwrap();
        assert!(all.iter().any(|r| r.id == id));

        assert!(delete_command(&conn, id).unwrap());
        assert!(get_command_by_id(&conn, id).unwrap().is_none());
        assert!(!delete_command(&conn, id).unwrap());
    }
}
