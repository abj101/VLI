//! SQLite command storage (`command_nodes`).

mod app_index;
mod models;
mod settings;

pub use app_index::{load_app_index, replace_app_index};
pub use models::{Action, CommandNode, NewCommandNode};
pub use settings::{
    apply_settings_patch, get_app_settings, get_setting, set_key_stored_flag, set_setting,
    AppSettings, SettingsPatch,
};

use rusqlite::Connection;
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    #[error("{0}")]
    Validation(String),
}

/// Open or create DB file, apply schema, remove legacy shipped sample rows (no auto-seed).
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
            fuzzy_threshold_pct INTEGER NOT NULL DEFAULT 80,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        ",
    )?;
    ensure_fuzzy_threshold_column(&conn)?;
    ensure_sort_order_column(&conn)?;
    app_index::ensure_app_index_schema(&conn)?;
    drop_legacy_ai_command_columns(&conn)?;
    purge_legacy_shipped_sample_commands(&conn)?;
    Ok(())
}

fn ensure_fuzzy_threshold_column(conn: &Connection) -> Result<(), DbError> {
    let mut stmt = conn.prepare("PRAGMA table_info(command_nodes)")?;
    let mut rows = stmt.query([])?;
    let mut has_column = false;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == "fuzzy_threshold_pct" {
            has_column = true;
            break;
        }
    }
    if !has_column {
        conn.execute(
            "ALTER TABLE command_nodes ADD COLUMN fuzzy_threshold_pct INTEGER NOT NULL DEFAULT 80",
            [],
        )?;
    }
    Ok(())
}

fn ensure_sort_order_column(conn: &Connection) -> Result<(), DbError> {
    let mut stmt = conn.prepare("PRAGMA table_info(command_nodes)")?;
    let mut rows = stmt.query([])?;
    let mut has_column = false;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == "sort_order" {
            has_column = true;
            break;
        }
    }
    if !has_column {
        conn.execute(
            "ALTER TABLE command_nodes ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    Ok(())
}

/// Phase 4: remove Haiku / `ai_mode` columns (SQLite 3.35+ `DROP COLUMN`).
fn drop_legacy_ai_command_columns(conn: &Connection) -> Result<(), DbError> {
    let mut stmt = conn.prepare("PRAGMA table_info(command_nodes)")?;
    let mut rows = stmt.query([])?;
    let mut has_ai_mode = false;
    let mut has_sub_prompt = false;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == "ai_mode" {
            has_ai_mode = true;
        } else if name == "sub_prompt" {
            has_sub_prompt = true;
        }
    }
    if has_ai_mode {
        conn.execute("ALTER TABLE command_nodes DROP COLUMN ai_mode", [])?;
    }
    if has_sub_prompt {
        conn.execute("ALTER TABLE command_nodes DROP COLUMN sub_prompt", [])?;
    }
    Ok(())
}

/// Previously shipped demo commands (exact `trigger_phrases` JSON as stored by serde_json).
fn legacy_shipped_sample_trigger_json() -> [&'static str; 4] {
    [
        r#"["open github and notepad"]"#,
        r#"["open notepad"]"#,
        r#"["open github"]"#,
        r#"["subprompt test"]"#,
    ]
}

fn purge_legacy_shipped_sample_commands(conn: &Connection) -> Result<(), DbError> {
    for tp in legacy_shipped_sample_trigger_json() {
        conn.execute(
            "DELETE FROM command_nodes WHERE trigger_phrases = ?1",
            [tp],
        )?;
    }
    Ok(())
}

pub fn insert_command(conn: &Connection, row: &NewCommandNode) -> Result<i64, DbError> {
    let trigger_phrases = serde_json::to_string(&row.trigger_phrases)?;
    let actions = serde_json::to_string(&row.actions)?;
    let enabled = i32::from(row.enabled);
    let fuzzy_threshold_pct = i64::from(row.fuzzy_threshold_pct.min(100));
    conn.execute(
        "INSERT INTO command_nodes (name, trigger_phrases, actions, enabled, fuzzy_threshold_pct) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![row.name, trigger_phrases, actions, enabled, fuzzy_threshold_pct,],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_all_commands(conn: &Connection) -> Result<Vec<CommandNode>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, trigger_phrases, actions, enabled, fuzzy_threshold_pct, created_at FROM command_nodes ORDER BY sort_order ASC, id ASC",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(row_to_command(row)?);
    }
    Ok(out)
}

#[allow(dead_code)] // reserved for Phase 2+ editor flows
pub fn update_command(conn: &Connection, id: i64, row: &NewCommandNode) -> Result<bool, DbError> {
    let trigger_phrases = serde_json::to_string(&row.trigger_phrases)?;
    let actions = serde_json::to_string(&row.actions)?;
    let enabled = i32::from(row.enabled);
    let fuzzy_threshold_pct = i64::from(row.fuzzy_threshold_pct.min(100));
    let n = conn.execute(
        "UPDATE command_nodes SET name = ?1, trigger_phrases = ?2, actions = ?3, enabled = ?4, fuzzy_threshold_pct = ?5 WHERE id = ?6",
        rusqlite::params![row.name, trigger_phrases, actions, enabled, fuzzy_threshold_pct, id],
    )?;
    Ok(n > 0)
}

#[allow(dead_code)] // used in tests and upcoming editor APIs
pub fn get_command_by_id(conn: &Connection, id: i64) -> Result<Option<CommandNode>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, trigger_phrases, actions, enabled, fuzzy_threshold_pct, created_at FROM command_nodes WHERE id = ?1",
    )?;
    let mut rows = stmt.query(rusqlite::params![id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row_to_command(row)?))
    } else {
        Ok(None)
    }
}

#[allow(dead_code)] // used in unit tests; reserved for Phase 3 editor
pub fn delete_command(conn: &Connection, id: i64) -> Result<bool, DbError> {
    let n = conn.execute("DELETE FROM command_nodes WHERE id = ?1", [id])?;
    Ok(n > 0)
}

pub fn reorder_commands(conn: &Connection, ordered_ids: &[i64]) -> Result<(), DbError> {
    if ordered_ids.is_empty() {
        return Ok(());
    }
    let mut seen = HashSet::with_capacity(ordered_ids.len());
    for (sort_order, id) in ordered_ids.iter().copied().enumerate() {
        if !seen.insert(id) {
            return Err(DbError::Validation(format!(
                "duplicate command id {id} in reorder payload"
            )));
        }
        let updated = conn.execute(
            "UPDATE command_nodes SET sort_order = ?1 WHERE id = ?2",
            rusqlite::params![sort_order as i64, id],
        )?;
        if updated == 0 {
            return Err(DbError::Validation(format!(
                "command with id {id} was not found"
            )));
        }
    }
    Ok(())
}

fn row_to_command(row: &rusqlite::Row<'_>) -> Result<CommandNode, DbError> {
    let trigger_phrases: String = row.get(2)?;
    let actions: String = row.get(3)?;
    let enabled_i: i32 = row.get(4)?;
    let fuzzy_threshold_pct: i64 = row.get(5)?;
    Ok(CommandNode {
        id: row.get(0)?,
        name: row.get(1)?,
        trigger_phrases: serde_json::from_str(&trigger_phrases)?,
        actions: serde_json::from_str(&actions)?,
        enabled: enabled_i != 0,
        fuzzy_threshold_pct: fuzzy_threshold_pct.clamp(0, 100) as u16,
        created_at: row.get(6)?,
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
    fn init_db_does_not_seed_commands() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("j.db");
        init_db(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        assert!(get_all_commands(&conn).unwrap().is_empty());

        init_db(&path).unwrap();
        let conn2 = Connection::open(&path).unwrap();
        assert!(get_all_commands(&conn2).unwrap().is_empty());
    }

    #[test]
    fn init_db_removes_legacy_shipped_sample_by_exact_trigger_json() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("legacy-samples.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            r"
            CREATE TABLE command_nodes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                trigger_phrases TEXT NOT NULL,
                actions TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                fuzzy_threshold_pct INTEGER NOT NULL DEFAULT 80,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            ",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO command_nodes (name, trigger_phrases, actions, enabled, fuzzy_threshold_pct) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                "Shipped sample",
                r#"["open github"]"#,
                r#"[{"open_url":{"url":"https://github.com"}}]"#,
                1,
                80
            ],
        )
        .unwrap();
        drop(conn);

        init_db(&path).unwrap();
        let after = Connection::open(&path).unwrap();
        assert!(
            get_all_commands(&after).unwrap().is_empty(),
            "legacy shipped sample row should be purged"
        );
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
                fuzzy_threshold_pct: 80,
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

    #[test]
    fn update_command_replaces_existing_values() {
        let (_dir, conn) = open_temp();
        let id = insert_command(
            &conn,
            &NewCommandNode {
                name: "Original".into(),
                trigger_phrases: vec!["do thing".into()],
                actions: vec![Action::OpenUrl {
                    url: "https://example.com".into(),
                }],
                enabled: true,
                fuzzy_threshold_pct: 80,
            },
        )
        .unwrap();

        let changed = update_command(
            &conn,
            id,
            &NewCommandNode {
                name: "Updated".into(),
                trigger_phrases: vec!["do better thing".into()],
                actions: vec![
                    Action::Wait { ms: 1200 },
                    Action::Speak {
                        text: "done".into(),
                    },
                ],
                enabled: false,
                fuzzy_threshold_pct: 90,
            },
        )
        .unwrap();
        assert!(changed);

        let one = get_command_by_id(&conn, id).unwrap().expect("row");
        assert_eq!(one.name, "Updated");
        assert_eq!(one.trigger_phrases, vec!["do better thing".to_string()]);
        assert!(!one.enabled);
        assert_eq!(one.fuzzy_threshold_pct, 90);
        assert_eq!(
            one.actions,
            vec![
                Action::Wait { ms: 1200 },
                Action::Speak {
                    text: "done".into()
                }
            ]
        );
    }

    #[test]
    fn can_read_pre_phase_two_actions_json_payload() {
        let (_dir, conn) = open_temp();
        conn.execute(
            "INSERT INTO command_nodes (name, trigger_phrases, actions, enabled) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                "Legacy",
                r#"["legacy phrase"]"#,
                r#"[{"open_app":{"name":"notepad","path":"notepad.exe"}}]"#,
                1
            ],
        )
        .unwrap();

        let all = get_all_commands(&conn).unwrap();
        assert!(all.iter().any(|node| {
            node.name == "Legacy"
                && node.fuzzy_threshold_pct == 80
                && node.actions
                    == vec![Action::OpenApp {
                        name: "notepad".into(),
                        path: "notepad.exe".into(),
                    }]
        }));
    }
}

#[cfg(test)]
mod update {
    use super::*;
    use rusqlite::Connection;
    use tempfile::tempdir;

    fn open_temp() -> (tempfile::TempDir, Connection) {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("update-test.db");
        init_db(&path).expect("init db");
        let conn = Connection::open(&path).expect("open db");
        (dir, conn)
    }

    #[test]
    fn round_trip_update_command() {
        let (_dir, conn) = open_temp();
        let id = insert_command(
            &conn,
            &NewCommandNode {
                name: "Original".into(),
                trigger_phrases: vec!["do thing".into()],
                actions: vec![Action::OpenUrl {
                    url: "https://example.com".into(),
                }],
                enabled: true,
                fuzzy_threshold_pct: 80,
            },
        )
        .expect("insert");

        let changed = update_command(
            &conn,
            id,
            &NewCommandNode {
                name: "Updated".into(),
                trigger_phrases: vec!["do better thing".into()],
                actions: vec![
                    Action::Wait { ms: 1200 },
                    Action::Speak {
                        text: "done".into(),
                    },
                ],
                enabled: false,
                fuzzy_threshold_pct: 90,
            },
        )
        .expect("update");
        assert!(changed);

        let one = get_command_by_id(&conn, id).expect("get").expect("row");
        assert_eq!(one.name, "Updated");
        assert_eq!(one.trigger_phrases, vec!["do better thing".to_string()]);
        assert!(!one.enabled);
        assert_eq!(one.fuzzy_threshold_pct, 90);
        assert_eq!(
            one.actions,
            vec![
                Action::Wait { ms: 1200 },
                Action::Speak {
                    text: "done".into()
                }
            ]
        );
    }
}

#[cfg(test)]
mod reorder {
    use super::*;
    use rusqlite::Connection;
    use std::fs;
    use tempfile::tempdir;

    fn open_temp() -> (tempfile::TempDir, Connection) {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("reorder-test.db");
        init_db(&path).expect("init db");
        let conn = Connection::open(&path).expect("open db");
        (dir, conn)
    }

    fn insert_named(conn: &Connection, name: &str) -> i64 {
        insert_command(
            conn,
            &NewCommandNode {
                name: name.to_string(),
                trigger_phrases: vec![format!("trigger {name}")],
                actions: vec![Action::Wait { ms: 10 }],
                enabled: true,
                fuzzy_threshold_pct: 80,
            },
        )
        .expect("insert")
    }

    #[test]
    fn sort_order_migration_adds_missing_column() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("legacy.db");
        let conn = Connection::open(&path).expect("open db");
        conn.execute_batch(
            r"
            CREATE TABLE command_nodes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                trigger_phrases TEXT NOT NULL,
                actions TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                fuzzy_threshold_pct INTEGER NOT NULL DEFAULT 80,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            ",
        )
        .expect("create legacy schema");
        drop(conn);

        init_db(&path).expect("run init db migration");
        init_db(&path).expect("run init db migration twice");

        let conn = Connection::open(&path).expect("open migrated db");
        let mut stmt = conn
            .prepare("PRAGMA table_info(command_nodes)")
            .expect("prepare table info");
        let mut rows = stmt.query([]).expect("query table info");
        let mut has_sort_order = false;
        while let Some(row) = rows.next().expect("next row") {
            let column_name: String = row.get(1).expect("column name");
            if column_name == "sort_order" {
                has_sort_order = true;
                break;
            }
        }

        assert!(
            has_sort_order,
            "sort_order column should exist after migration"
        );
    }

    #[test]
    fn reorder_commands_updates_display_order() {
        let (_dir, conn) = open_temp();
        let first = insert_named(&conn, "first");
        let second = insert_named(&conn, "second");
        let third = insert_named(&conn, "third");

        reorder_commands(&conn, &[third, first, second]).expect("reorder");

        let all = get_all_commands(&conn).expect("list commands");
        let ids: Vec<i64> = all
            .iter()
            .filter(|node| [first, second, third].contains(&node.id))
            .map(|node| node.id)
            .collect();
        assert_eq!(ids, vec![third, first, second]);
    }

    #[test]
    fn migration_runs_on_copied_pre_phase_three_db_file() {
        let dir = tempdir().expect("tempdir");
        let source_path = dir.path().join("phase2.db");
        let copied_path = dir.path().join("phase2-copy.db");

        let source = Connection::open(&source_path).expect("open source db");
        source
            .execute_batch(
                r"
                CREATE TABLE command_nodes (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL,
                    trigger_phrases TEXT NOT NULL,
                    actions TEXT NOT NULL,
                    enabled INTEGER NOT NULL DEFAULT 1,
                    fuzzy_threshold_pct INTEGER NOT NULL DEFAULT 80,
                    created_at TEXT NOT NULL DEFAULT (datetime('now'))
                );
                ",
            )
            .expect("create phase2 schema");
        source
            .execute(
                "INSERT INTO command_nodes (name, trigger_phrases, actions, enabled, fuzzy_threshold_pct) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    "Legacy Node",
                    r#"["legacy trigger"]"#,
                    r#"[{"wait":{"ms":50}}]"#,
                    1,
                    80
                ],
            )
            .expect("insert legacy row");
        drop(source);

        fs::copy(&source_path, &copied_path).expect("copy phase2 db file");
        init_db(&copied_path).expect("migrate copied db");

        let migrated = Connection::open(&copied_path).expect("open migrated db");
        let rows = get_all_commands(&migrated).expect("load commands after migration");
        assert!(
            rows.iter().any(|node| node.name == "Legacy Node"),
            "legacy row should remain after migration"
        );

        let mut stmt = migrated
            .prepare("PRAGMA table_info(command_nodes)")
            .expect("prepare table info");
        let mut pragma_rows = stmt.query([]).expect("query table info");
        let mut has_sort_order = false;
        while let Some(row) = pragma_rows.next().expect("next row") {
            let column_name: String = row.get(1).expect("column name");
            if column_name == "sort_order" {
                has_sort_order = true;
                break;
            }
        }
        assert!(
            has_sort_order,
            "sort_order column should exist after migration"
        );
    }
}
