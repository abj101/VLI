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
    seed_builtin_run_registered_script(conn)?;
    seed_builtin_web_search(conn)?;
    seed_builtin_shortcut(conn)?;
    seed_builtin_media(conn)?;
    seed_builtin_type_text(conn)?;
    seed_builtin_say(conn)?;
    seed_builtin_desktop_tools(conn)?;
    migrate_builtin_descriptions(conn)?;
    migrate_builtin_shortcut_enum(conn)?;
    Ok(())
}

/// Builtin shortcut names, in the order d1 sees them; must match `execute_shortcut_tool`.
const SHORTCUT_NAMES: [&str; 20] = [
    "copy",
    "paste",
    "cut",
    "undo",
    "redo",
    "select_all",
    "save",
    "find",
    "browser_back",
    "browser_forward",
    "new_tab",
    "close_tab",
    "reopen_tab",
    "next_tab",
    "previous_tab",
    "refresh",
    "close_window",
    "switch_app",
    "minimize",
    "show_desktop",
];

/// Older installs stored a different list (missing values, stale ones like `hard_refresh`, other
/// order). d1 answers by option position, so a drifted list routes "new tab" to `next_tab`.
fn migrate_builtin_shortcut_enum(conn: &Connection) -> Result<(), DbError> {
    let Some(tool) = get_tool_by_name(conn, "shortcut")? else {
        return Ok(());
    };
    if !tool.builtin {
        return Ok(());
    }
    let mut parameters = tool.parameters.clone();
    let Some(param) = parameters.iter_mut().find(|p| p.name == "name") else {
        return Ok(());
    };
    if param.enum_values != SHORTCUT_NAMES {
        param.enum_values = SHORTCUT_NAMES.into_iter().map(str::to_string).collect();
        update_tool(
            conn,
            tool.id,
            &NewToolDefinition {
                name: tool.name,
                display_name: tool.display_name,
                description: tool.description,
                parameters,
                actions: tool.actions,
                enabled: tool.enabled,
                builtin: tool.builtin,
            },
        )?;
    }
    Ok(())
}

fn migrate_builtin_descriptions(conn: &Connection) -> Result<(), DbError> {
    for (name, old, updated) in [
        (
            "media",
            "Controls media playback and volume",
            "play/pause music, next or previous song/track, volume up/down, mute",
        ),
        (
            "open_target",
            "Opens an installed app or browser URL from a spoken name",
            "open or launch an app or program by name, optionally on the left/right",
        ),
        (
            "open_url",
            "Opens a URL in the default browser",
            "go to an explicit website address like github.com",
        ),
        (
            "run_registered_script",
            "Runs an allowlisted user script by id with declared arguments only",
            "run one of the user's saved scripts by id",
        ),
        (
            "say",
            "Speaks the requested text aloud",
            "Jarvis should say, speak or read some words aloud",
        ),
        (
            "say",
            "Jarvis should speak some words aloud",
            "Jarvis should say, speak or read some words aloud",
        ),
        (
            "shortcut",
            "Runs a common keyboard shortcut",
            "copy, cut, paste, undo, redo, select all, save, find on this page, browser back/forward, new/close/reopen/next/previous tab, refresh, close window, switch app, minimize or hide window, show desktop",
        ),
        (
            "shortcut",
            "copy, paste, undo, save, find, new/close/reopen/next tab, refresh, close window, switch app, minimize, show desktop",
            "copy, cut, paste, undo, redo, select all, save, find on this page, browser back/forward, new/close/reopen/next/previous tab, refresh, close window, switch app, minimize or hide window, show desktop",
        ),
        (
            "snap_window",
            "Moves the focused window to a screen zone",
            "snap the current window to the left half or right half of the screen, or maximize it (not minimize or hide)",
        ),
        (
            "snap_window",
            "move/snap/maximize the current window",
            "snap the current window to the left half or right half of the screen, or maximize it (not minimize or hide)",
        ),
        (
            "type_text",
            "Types the requested text into the focused app",
            "type, write or enter dictated words into the current app",
        ),
        (
            "type_text",
            "type or write dictated words into the current app",
            "type, write or enter dictated words into the current app",
        ),
        (
            "web_search",
            "Searches Google for the requested query",
            "search the internet, google or look something up online (not finding text on this page)",
        ),
        (
            "web_search",
            "search, google or look something up on the web",
            "search the internet, google or look something up online (not finding text on this page)",
        ),
        ("shortcut", "copy, cut, paste, undo, redo, select all, save, find on this page, new/close/reopen/next/previous tab, refresh, close window, switch app, minimize or hide window, show desktop", "copy, cut, paste, undo, redo, select all, save, find on this page, browser back/forward, new/close/reopen/next/previous tab, refresh, close window, switch app, minimize or hide window, show desktop"),
    ] {
        conn.execute(
            "UPDATE tools SET description = ?1 WHERE name = ?2 AND builtin = 1 AND description = ?3",
            rusqlite::params![updated, name, old],
        )?;
    }
    Ok(())
}

fn seed_builtin_web_search(conn: &Connection) -> Result<(), DbError> {
    seed_tool_if_missing(
        conn,
        NewToolDefinition {
            name: "web_search".into(),
            display_name: "Web search".into(),
            description: "search the internet, google or look something up online (not finding text on this page)".into(),
            parameters: vec![ToolParameter {
                name: "query".into(),
                param_type: "string".into(),
                description: Some("Search terms".into()),
                required: true,
                enum_values: vec![],
            }],
            actions: vec![Action::OpenUrl {
                url: "https://www.google.com/search?q={{query}}".into(),
            }],
            enabled: true,
            builtin: true,
        },
    )
}

fn seed_builtin_shortcut(conn: &Connection) -> Result<(), DbError> {
    seed_tool_if_missing(
        conn,
        NewToolDefinition {
            name: "shortcut".into(),
            display_name: "Keyboard shortcut".into(),
            description: "copy, cut, paste, undo, redo, select all, save, find on this page, browser back/forward, new/close/reopen/next/previous tab, refresh, close window, switch app, minimize or hide window, show desktop".into(),
            parameters: vec![ToolParameter {
                name: "name".into(),
                param_type: "enum".into(),
                description: Some("Shortcut to run".into()),
                required: true,
                enum_values: SHORTCUT_NAMES.into_iter().map(str::to_string).collect(),
            }],
            actions: vec![Action::SendKeys {
                keys: "{{name}}".into(),
            }],
            enabled: true,
            builtin: true,
        },
    )
}

fn seed_builtin_media(conn: &Connection) -> Result<(), DbError> {
    seed_tool_if_missing(
        conn,
        NewToolDefinition {
            name: "media".into(),
            display_name: "Media control".into(),
            description: "play/pause music, next or previous song/track, volume up/down, mute"
                .into(),
            parameters: vec![ToolParameter {
                name: "control".into(),
                param_type: "enum".into(),
                description: Some("Media action".into()),
                required: true,
                enum_values: [
                    "play_pause",
                    "next",
                    "previous",
                    "volume_up",
                    "volume_down",
                    "mute",
                ]
                .into_iter()
                .map(str::to_string)
                .collect(),
            }],
            actions: vec![Action::SendKeys {
                keys: "{{control}}".into(),
            }],
            enabled: true,
            builtin: true,
        },
    )
}

fn seed_builtin_type_text(conn: &Connection) -> Result<(), DbError> {
    seed_tool_if_missing(
        conn,
        NewToolDefinition {
            name: "type_text".into(),
            display_name: "Type text".into(),
            description: "type, write or enter dictated words into the current app".into(),
            parameters: vec![ToolParameter {
                name: "text".into(),
                param_type: "string".into(),
                description: Some("Text to type".into()),
                required: true,
                enum_values: vec![],
            }],
            actions: vec![Action::SendKeys {
                keys: "{{text}}".into(),
            }],
            enabled: true,
            builtin: true,
        },
    )
}

fn seed_builtin_say(conn: &Connection) -> Result<(), DbError> {
    seed_tool_if_missing(
        conn,
        NewToolDefinition {
            name: "say".into(),
            display_name: "Speak text".into(),
            description: "Jarvis should say, speak or read some words aloud".into(),
            parameters: vec![ToolParameter {
                name: "text".into(),
                param_type: "string".into(),
                description: Some("Text to speak".into()),
                required: true,
                enum_values: vec![],
            }],
            actions: vec![Action::Speak {
                text: "{{text}}".into(),
            }],
            enabled: true,
            builtin: true,
        },
    )
}

/// Keystrokes, lock, close-by-name and side focus. Their `actions` are placeholders where
/// `execute_tool` runs them natively, like `shortcut`.
fn seed_builtin_desktop_tools(conn: &Connection) -> Result<(), DbError> {
    let string_param = |name: &str, description: &str| ToolParameter {
        name: name.into(),
        param_type: "string".into(),
        description: Some(description.into()),
        required: true,
        enum_values: vec![],
    };
    for (name, display_name, description, parameters, keys) in [
        (
            "press_keys",
            "Press keys",
            "press a key or key combination by name, like alt tab, control z or press enter",
            vec![string_param("keys", "SendKeys string, e.g. %{TAB}")],
            "{{keys}}",
        ),
        (
            "lock_computer",
            "Lock computer",
            "lock the computer or lock the screen",
            vec![],
            "",
        ),
        (
            "close_app",
            "Close app",
            "close or quit an app by its name, like close spotify",
            vec![string_param("target", "App name to close")],
            "",
        ),
        (
            "focus_side",
            "Focus side window",
            "switch to the window tiled on the left or right side of the screen",
            vec![ToolParameter {
                name: "side".into(),
                param_type: "enum".into(),
                description: Some("Screen side".into()),
                required: true,
                enum_values: vec!["left".into(), "right".into()],
            }],
            "",
        ),
    ] {
        seed_tool_if_missing(
            conn,
            NewToolDefinition {
                name: name.into(),
                display_name: display_name.into(),
                description: description.into(),
                parameters,
                actions: vec![Action::SendKeys { keys: keys.into() }],
                enabled: true,
                builtin: true,
            },
        )?;
    }
    Ok(())
}

fn seed_tool_if_missing(conn: &Connection, row: NewToolDefinition) -> Result<(), DbError> {
    if get_tool_by_name(conn, &row.name)?.is_none() {
        insert_tool(conn, &row)?;
    }
    Ok(())
}

fn seed_builtin_run_registered_script(conn: &Connection) -> Result<(), DbError> {
    let parameters = vec![
        ToolParameter {
            name: "script_id".into(),
            param_type: "string".into(),
            description: Some("Registered script id from the allowlist".into()),
            required: true,
            enum_values: vec![],
        },
        ToolParameter {
            name: "args".into(),
            param_type: "string".into(),
            description: Some("Optional JSON array of string arguments, e.g. [\"staging\"]".into()),
            required: false,
            enum_values: vec![],
        },
    ];
    let actions = vec![Action::RunRegisteredScript {
        script_id: "{{script_id}}".into(),
        args: vec![],
    }];
    let row = NewToolDefinition {
        name: "run_registered_script".into(),
        display_name: "Run registered script".into(),
        description: "run one of the user's saved scripts by id".into(),
        parameters,
        actions,
        enabled: true,
        builtin: true,
    };
    if get_tool_by_name(conn, "run_registered_script")?.is_none() {
        insert_tool(conn, &row)?;
    }
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
            description: Some("Optional window zone: left_half, right_half, maximize".into()),
            required: false,
            enum_values: vec!["left_half".into(), "right_half".into(), "maximize".into()],
        },
    ];
    let actions = vec![Action::OpenTarget {
        target: "{{target}}".into(),
        placement: Some("{{placement}}".into()),
    }];
    let row = NewToolDefinition {
        name: "open_target".into(),
        display_name: "Open app or site".into(),
        description: "open or launch an app or program by name, optionally on the left/right"
            .into(),
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
        description: "go to an explicit website address like github.com".into(),
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
        enum_values: vec!["left_half".into(), "right_half".into(), "maximize".into()],
    }];
    let actions = vec![Action::PlaceWindow {
        zone: "{{zone}}".into(),
        monitor: Some("monitor_primary".into()),
    }];
    let row = NewToolDefinition {
        name: "snap_window".into(),
        display_name: "Snap window".into(),
        description: "snap the current window to the left half or right half of the screen, or maximize it (not minimize or hide)".into(),
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
        return Err(DbError::Validation(
            "at least one action is required".into(),
        ));
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
    fn init_db_seeds_decider_builtins_and_shortcut_enums() {
        let (_dir, conn) = open_temp();
        for name in ["web_search", "shortcut", "media", "type_text", "say"] {
            let tool = get_tool_by_name(&conn, name)
                .unwrap()
                .expect("builtin tool");
            assert!(tool.builtin && tool.enabled);
            assert_eq!(tool.parameters.len(), 1);
        }
        let shortcut = get_tool_by_name(&conn, "shortcut").unwrap().unwrap();
        assert!(shortcut.parameters[0]
            .enum_values
            .iter()
            .any(|value| value == "minimize"));
        assert!(shortcut.parameters[0]
            .enum_values
            .iter()
            .any(|value| value == "show_desktop"));
    }

    #[test]
    fn builtin_description_migration_preserves_user_edits() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_tools_schema(&conn).unwrap();
        for (name, description) in [
            ("media", "Controls media playback and volume"),
            (
                "open_target",
                "Opens an installed app or browser URL from a spoken name",
            ),
            ("open_url", "Opens a URL in the default browser"),
            (
                "run_registered_script",
                "Runs an allowlisted user script by id with declared arguments only",
            ),
            ("say", "Speaks the requested text aloud"),
            ("shortcut", "Runs a common keyboard shortcut"),
            ("snap_window", "Moves the focused window to a screen zone"),
            ("type_text", "Types the requested text into the focused app"),
            ("web_search", "Searches Google for the requested query"),
        ] {
            conn.execute(
                "UPDATE tools SET description = ?1 WHERE name = ?2",
                rusqlite::params![description, name],
            )
            .unwrap();
        }
        conn.execute(
            "UPDATE tools SET description = 'my custom search description' WHERE name = 'web_search'",
            [],
        )
        .unwrap();

        ensure_tools_schema(&conn).unwrap();

        let expected = [
            (
                "media",
                "play/pause music, next or previous song/track, volume up/down, mute",
            ),
            (
                "open_target",
                "open or launch an app or program by name, optionally on the left/right",
            ),
            ("open_url", "go to an explicit website address like github.com"),
            (
                "run_registered_script",
                "run one of the user's saved scripts by id",
            ),
            ("say", "Jarvis should say, speak or read some words aloud"),
            (
                "shortcut",
                "copy, cut, paste, undo, redo, select all, save, find on this page, browser back/forward, new/close/reopen/next/previous tab, refresh, close window, switch app, minimize or hide window, show desktop",
            ),
            (
                "snap_window",
                "snap the current window to the left half or right half of the screen, or maximize it (not minimize or hide)",
            ),
            ("type_text", "type, write or enter dictated words into the current app"),
        ];
        for (name, description) in expected {
            assert_eq!(
                get_tool_by_name(&conn, name).unwrap().unwrap().description,
                description,
                "migrate {name} description",
            );
        }
        assert_eq!(
            get_tool_by_name(&conn, "web_search")
                .unwrap()
                .unwrap()
                .description,
            "my custom search description"
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
                actions: vec![Action::Speak { text: "bye".into() }],
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
