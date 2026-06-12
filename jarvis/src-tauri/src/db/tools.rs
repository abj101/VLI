//! SQLite storage for the tool registry (`tools` table).

use super::models::{Action, NewToolDefinition, ToolDefinition, ToolParameter};
use super::DbError;
use rusqlite::Connection;

pub fn ensure_tools_schema(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(
        r"
        CREATE TABLE IF NOT EXISTS tools (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            display_name TEXT NOT NULL,
            description TEXT NOT NULL,
            parameters TEXT NOT NULL,
            actions TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            builtin INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        ",
    )?;
    seed_builtin_open_url(conn)?;
    seed_builtin_open_target(conn)?;
    seed_builtin_snap_window(conn)?;
    Ok(())
}

fn seed_builtin_open_target(conn: &Connection) -> Result<(), DbError> {
    let parameters = vec![
        ToolParameter {
            name: "target".into(),
            param_type: "string".into(),
            description: Some("Spoken app or site name to open".into()),
            required: true,
            enum_values: vec![],
        },
        ToolParameter {
            name: "placement".into(),
            param_type: "enum".into(),
            description: Some(
                "Optional window zone: left_half, right_half, maximize".into(),
            ),
            required: false,
            enum_values: vec![
                "left_half".into(),
                "right_half".into(),
                "maximize".into(),
            ],
        },
    ];
    let actions = vec![Action::OpenTarget {
        target: "{{target}}".into(),
        placement: Some("{{placement}}".into()),
    }];
    let row = NewToolDefinition {
        name: "open_target".into(),
        display_name: "Open app or site".into(),
        description: "Opens an installed app or browser URL from a spoken name".into(),
        parameters,
        actions,
        enabled: true,
        builtin: true,
    };
    if get_tool_by_name(conn, "open_target")?.is_none() {
        insert_tool(conn, &row)?;
    }
    Ok(())
}

fn seed_builtin_open_url(conn: &Connection) -> Result<(), DbError> {
    let parameters = vec![ToolParameter {
        name: "url".into(),
        param_type: "string".into(),
        description: Some("HTTP or HTTPS URL to open in the default browser".into()),
        required: true,
        enum_values: vec![],
    }];
    let actions = vec![Action::OpenUrl {
        url: "{{url}}".into(),
    }];
    let row = NewToolDefinition {
        name: "open_url".into(),
        display_name: "Open URL".into(),
        description: "Opens a URL in the default browser".into(),
        parameters,
        actions,
        enabled: true,
        builtin: true,
    };
    if get_tool_by_name(conn, "open_url")?.is_none() {
        insert_tool(conn, &row)?;
    }
    Ok(())
}

fn seed_builtin_snap_window(conn: &Connection) -> Result<(), DbError> {
    let parameters = vec![ToolParameter {
        name: "zone".into(),
        param_type: "enum".into(),
        description: Some("Window zone for the focused window".into()),
        required: true,
        enum_values: vec![
            "left_half".into(),
            "right_half".into(),
            "maximize".into(),
        ],
    }];
    let actions = vec![Action::PlaceWindow {
        zone: "{{zone}}".into(),
        monitor: Some("monitor_primary".into()),
    }];
    let row = NewToolDefinition {
        name: "snap_window".into(),
        display_name: "Snap window".into(),
        description: "Moves the focused window to a screen zone".into(),
        parameters,
        actions,
        enabled: true,
        builtin: true,
    };
    if get_tool_by_name(conn, "snap_window")?.is_none() {
        insert_tool(conn, &row)?;
    }
    Ok(())
}

pub fn insert_tool(conn: &Connection, row: &NewToolDefinition) -> Result<i64, DbError> {
    validate_new_tool(row)?;
    let parameters = serde_json::to_string(&row.parameters)?;
    let actions = serde_json::to_string(&row.actions)?;
    let enabled = i32::from(row.enabled);
    let builtin = i32::from(row.builtin);
    conn.execute(
        "INSERT INTO tools (name, display_name, description, parameters, actions, enabled, builtin) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            row.name,
            row.display_name,
            row.description,
            parameters,
            actions,
            enabled,
            builtin,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_all_tools(conn: &Connection) -> Result<Vec<ToolDefinition>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, display_name, description, parameters, actions, enabled, builtin, created_at FROM tools ORDER BY builtin DESC, name ASC",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(row_to_tool(row)?);
    }
    Ok(out)
}

pub fn get_tool_by_id(conn: &Connection, id: i64) -> Result<Option<ToolDefinition>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, display_name, description, parameters, actions, enabled, builtin, created_at FROM tools WHERE id = ?1",
    )?;
    let mut rows = stmt.query(rusqlite::params![id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row_to_tool(row)?))
    } else {
        Ok(None)
    }
}

pub fn get_tool_by_name(conn: &Connection, name: &str) -> Result<Option<ToolDefinition>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, display_name, description, parameters, actions, enabled, builtin, created_at FROM tools WHERE name = ?1",
    )?;
    let mut rows = stmt.query(rusqlite::params![name])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row_to_tool(row)?))
    } else {
        Ok(None)
    }
}

pub fn update_tool(conn: &Connection, id: i64, row: &NewToolDefinition) -> Result<bool, DbError> {
    validate_new_tool(row)?;
    let parameters = serde_json::to_string(&row.parameters)?;
    let actions = serde_json::to_string(&row.actions)?;
    let enabled = i32::from(row.enabled);
    let builtin = i32::from(row.builtin);
    let n = conn.execute(
        "UPDATE tools SET name = ?1, display_name = ?2, description = ?3, parameters = ?4, actions = ?5, enabled = ?6, builtin = ?7 WHERE id = ?8",
        rusqlite::params![
            row.name,
            row.display_name,
            row.description,
            parameters,
            actions,
            enabled,
            builtin,
            id,
        ],
    )?;
    Ok(n > 0)
}

pub fn delete_tool(conn: &Connection, id: i64) -> Result<bool, DbError> {
    let n = conn.execute("DELETE FROM tools WHERE id = ?1", [id])?;
    Ok(n > 0)
}

fn row_to_tool(row: &rusqlite::Row<'_>) -> Result<ToolDefinition, DbError> {
    let parameters: String = row.get(4)?;
    let actions: String = row.get(5)?;
    let enabled_i: i32 = row.get(6)?;
    let builtin_i: i32 = row.get(7)?;
    Ok(ToolDefinition {
        id: row.get(0)?,
        name: row.get(1)?,
        display_name: row.get(2)?,
        description: row.get(3)?,
        parameters: serde_json::from_str(&parameters)?,
        actions: serde_json::from_str(&actions)?,
        enabled: enabled_i != 0,
        builtin: builtin_i != 0,
        created_at: row.get(8)?,
    })
}

fn validate_new_tool(row: &NewToolDefinition) -> Result<(), DbError> {
    let name = row.name.trim();
    if name.is_empty() {
        return Err(DbError::Validation("tool name is required".into()));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(DbError::Validation(
            "tool name must be alphanumeric with underscores or hyphens".into(),
        ));
    }
    if row.display_name.trim().is_empty() {
        return Err(DbError::Validation("display_name is required".into()));
    }
    if row.actions.is_empty() {
        return Err(DbError::Validation("at least one action is required".into()));
    }
    let mut seen = std::collections::HashSet::new();
    for param in &row.parameters {
        let key = param.name.trim();
        if key.is_empty() {
            return Err(DbError::Validation("parameter name is required".into()));
        }
        if !seen.insert(key.to_string()) {
            return Err(DbError::Validation(format!(
                "duplicate parameter name `{key}`"
            )));
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
        let path = dir.path().join("tools-test.db");
        init_db(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        (dir, conn)
    }

    #[test]
    fn init_db_seeds_open_target_builtin() {
        let (_dir, conn) = open_temp();
        let tool = get_tool_by_name(&conn, "open_target")
            .unwrap()
            .expect("builtin open_target");
        assert!(tool.builtin);
        assert!(tool.enabled);
        assert_eq!(tool.parameters.len(), 2);
        assert_eq!(tool.parameters[0].name, "target");
        assert!(matches!(
            tool.actions.first(),
            Some(Action::OpenTarget { .. })
        ));
    }

    #[test]
    fn init_db_seeds_snap_window_builtin() {
        let (_dir, conn) = open_temp();
        let tool = get_tool_by_name(&conn, "snap_window")
            .unwrap()
            .expect("builtin snap_window");
        assert!(tool.builtin);
        assert_eq!(tool.parameters[0].name, "zone");
        assert!(matches!(
            tool.actions.first(),
            Some(Action::PlaceWindow { .. })
        ));
    }

    #[test]
    fn init_db_seeds_open_url_builtin() {
        let (dir, conn) = open_temp();
        let path = dir.path().join("tools-test.db");
        let tool = get_tool_by_name(&conn, "open_url")
            .unwrap()
            .expect("builtin open_url");
        assert!(tool.builtin);
        assert!(tool.enabled);
        assert_eq!(tool.parameters.len(), 1);
        assert_eq!(tool.parameters[0].name, "url");
        assert_eq!(
            tool.actions,
            vec![Action::OpenUrl {
                url: "{{url}}".into()
            }]
        );

        drop(conn);
        init_db(&path).unwrap();
        let conn2 = Connection::open(&path).unwrap();
        let all = get_all_tools(&conn2).unwrap();
        assert_eq!(
            all.iter().filter(|t| t.name == "open_url").count(),
            1,
            "re-init must not duplicate builtin"
        );
    }

    #[test]
    fn tool_crud_round_trip() {
        let (_dir, conn) = open_temp();
        let id = insert_tool(
            &conn,
            &NewToolDefinition {
                name: "speak_test".into(),
                display_name: "Speak Test".into(),
                description: "Says hello".into(),
                parameters: vec![ToolParameter {
                    name: "text".into(),
                    param_type: "string".into(),
                    description: None,
                    required: true,
                    enum_values: vec![],
                }],
                actions: vec![Action::Speak {
                    text: "{{text}}".into(),
                }],
                enabled: true,
                builtin: false,
            },
        )
        .unwrap();

        let listed = get_all_tools(&conn).unwrap();
        assert!(listed.iter().any(|t| t.id == id && t.name == "speak_test"));

        let updated = update_tool(
            &conn,
            id,
            &NewToolDefinition {
                name: "speak_test".into(),
                display_name: "Speak Updated".into(),
                description: "Says goodbye".into(),
                parameters: vec![],
                actions: vec![Action::Speak {
                    text: "bye".into(),
                }],
                enabled: false,
                builtin: false,
            },
        )
        .unwrap();
        assert!(updated);

        let one = get_tool_by_id(&conn, id).unwrap().expect("row");
        assert_eq!(one.display_name, "Speak Updated");
        assert!(!one.enabled);

        assert!(delete_tool(&conn, id).unwrap());
        assert!(get_tool_by_id(&conn, id).unwrap().is_none());
    }
}
