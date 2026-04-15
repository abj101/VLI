//! SQLite cache for the installed-app index.

use crate::apps::AppEntry;
use rusqlite::Connection;

use super::DbError;

pub(crate) fn ensure_app_index_schema(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(
        r"
        CREATE TABLE IF NOT EXISTS app_index (
            exe_path TEXT PRIMARY KEY,
            display_name TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS app_index_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        ",
    )?;
    Ok(())
}

pub fn load_app_index(conn: &Connection) -> Result<Vec<AppEntry>, DbError> {
    let mut stmt = conn.prepare("SELECT exe_path, display_name FROM app_index")?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(AppEntry {
            exe_path: row.get(0)?,
            display_name: row.get(1)?,
            icon_data_url: None,
        });
    }
    Ok(out)
}

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
            "INSERT INTO app_index (exe_path, display_name) VALUES (?1, ?2)",
            rusqlite::params![e.exe_path, e.display_name],
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
                icon_data_url: None,
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
    }
}
