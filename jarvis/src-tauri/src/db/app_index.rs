//! SQLite cache for the installed-app index.

use crate::apps::AppEntry;
use rusqlite::Connection;

use super::DbError;

pub(crate) fn ensure_app_index_schema(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(
        r"
        CREATE TABLE IF NOT EXISTS app_index (
            exe_path TEXT PRIMARY KEY,
            display_name TEXT NOT NULL,
            icon_data_url TEXT
        );
        CREATE TABLE IF NOT EXISTS app_index_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        ",
    )?;
    ensure_app_index_icon_column(conn)?;
    Ok(())
}

fn ensure_app_index_icon_column(conn: &Connection) -> Result<(), DbError> {
    let mut stmt = conn.prepare("PRAGMA table_info(app_index)")?;
    let mut rows = stmt.query([])?;
    let mut has_icon_data_url = false;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == "icon_data_url" {
            has_icon_data_url = true;
            break;
        }
    }
    if !has_icon_data_url {
        conn.execute("ALTER TABLE app_index ADD COLUMN icon_data_url TEXT", [])?;
    }
    Ok(())
}

pub fn load_app_index(conn: &Connection) -> Result<Vec<AppEntry>, DbError> {
    let mut stmt = conn.prepare("SELECT exe_path, display_name, icon_data_url FROM app_index")?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(AppEntry {
            exe_path: row.get(0)?,
            display_name: row.get(1)?,
            icon_data_url: row.get(2)?,
        });
    }
    Ok(out)
}

#[cfg(test)]
pub fn get_app_index_last_scan_unix(conn: &Connection) -> Result<Option<i64>, DbError> {
    let mut stmt = conn.prepare("SELECT value FROM app_index_meta WHERE key = 'last_scan_unix'")?;
    let mut rows = stmt.query([])?;
    if let Some(row) = rows.next()? {
        let v: String = row.get(0)?;
        Ok(v.parse().ok())
    } else {
        Ok(None)
    }
}

pub fn replace_app_index(
    conn: &Connection,
    entries: &[AppEntry],
    scan_unix: i64,
) -> Result<(), DbError> {
    conn.execute_batch("BEGIN IMMEDIATE;")?;
    conn.execute("DELETE FROM app_index", [])?;
    for e in entries {
        conn.execute(
            "INSERT INTO app_index (exe_path, display_name, icon_data_url) VALUES (?1, ?2, ?3)",
            rusqlite::params![e.exe_path, e.display_name, e.icon_data_url],
        )?;
    }
    conn.execute(
        "INSERT INTO app_index_meta (key, value) VALUES ('last_scan_unix', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [scan_unix.to_string()],
    )?;
    conn.execute_batch("COMMIT;")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;
    use tempfile::tempdir;

    #[test]
    fn roundtrip_app_index() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("t.db");
        init_db(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        let entries = vec![
            AppEntry {
                exe_path: r"C:\a\b.exe".into(),
                display_name: "Bee".into(),
                icon_data_url: Some("data:image/png;base64,AAA=".into()),
            },
            AppEntry {
                exe_path: r"C:\c\d.exe".into(),
                display_name: "Dee".into(),
                icon_data_url: None,
            },
        ];
        replace_app_index(&conn, &entries, 12345).unwrap();
        assert_eq!(get_app_index_last_scan_unix(&conn).unwrap(), Some(12345));
        let loaded = load_app_index(&conn).unwrap();
        assert_eq!(loaded.len(), 2);
        assert!(loaded.iter().any(|e| e.display_name == "Bee"));
        assert_eq!(
            loaded
                .iter()
                .find(|e| e.display_name == "Bee")
                .and_then(|e| e.icon_data_url.as_deref()),
            Some("data:image/png;base64,AAA=")
        );
    }

    #[test]
    fn ensure_app_index_schema_adds_icon_column_for_legacy_db() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r"
            CREATE TABLE app_index (
                exe_path TEXT PRIMARY KEY,
                display_name TEXT NOT NULL
            );
            ",
        )
        .unwrap();
        ensure_app_index_schema(&conn).unwrap();

        let mut stmt = conn.prepare("PRAGMA table_info(app_index)").unwrap();
        let mut rows = stmt.query([]).unwrap();
        let mut has_icon_column = false;
        while let Some(row) = rows.next().unwrap() {
            let name: String = row.get(1).unwrap();
            if name == "icon_data_url" {
                has_icon_column = true;
                break;
            }
        }
        assert!(has_icon_column, "icon_data_url column should exist");
    }
}
