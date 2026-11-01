//! d1 systemone decision router.
use crate::{
    db::{get_registered_script, ToolDefinition},
    llm::{
        decider_args::extract_args,
        router::{
            RouterError, RouterErrorCode, RouterRouteResult, RouterToolCall, MAX_ROUTER_TOOL_CALLS,
        },
    },
};
use rusqlite::Connection;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

const NONE_HINT: &str = "chit-chat, filler, or nonsense that asks the computer to do nothing";

/// Coarse type of the focused app, read from the app name in the context line.
fn app_kind(context: &str) -> Option<&'static str> {
    let app = context
        .split(" (window:")
        .next()
        .unwrap_or(context)
        .to_lowercase();
    const KINDS: &[(&str, &[&str])] = &[
        ("code editor", &["visual studio code", "vs code", "cursor", "zed", "sublime", "notepad++"]),
        ("browser", &["chrome", "edge", "firefox", "brave", "opera", "vivaldi"]),
        ("music player", &["spotify", "music", "itunes", "winamp", "vlc"]),
        ("file manager", &["file explorer", "explorer", "finder"]),
        ("document editor", &["word", "notepad", "wordpad", "writer"]),
    ];
    KINDS
        .iter()
        .find(|(_, names)| names.iter().any(|name| app.contains(name)))
        .map(|(kind, _)| *kind)
}

/// Actions whose meaning depends on the focused app. d1 picks the plain meaning;
/// `bind_unspecified` turns it into a real tool call for the app type.
const UNSPECIFIED: [(&str, &str); 5] = [
    ("go_back", "go back, back up or one step back, without naming a song, page or tab"),
    ("go_next", "go to the next one, without naming a song or tab"),
    ("close_this", "close or get rid of this, without naming a tab, window or app"),
    ("left_one", "the one on the left, without naming a tab or window"),
    ("right_one", "the one on the right, without naming a tab or window"),
];

/// True when every word of the script's name (or id) is spoken, allowing small slips.
fn names_script(transcript: &str, display_name: &str, id: &str) -> bool {
    let spoken: Vec<String> = transcript
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    [display_name.to_lowercase(), id.to_lowercase().replace(['_', '-'], " ")]
        .iter()
        .any(|name| {
            let words: Vec<&str> = name.split_whitespace().filter(|w| w.len() > 2).collect();
            !words.is_empty()
                && words.iter().all(|word| {
                    spoken
                        .iter()
                        .any(|s| rapidfuzz::fuzz::ratio(word.chars(), s.chars()) >= 0.8)
                })
        })
}

fn bind_unspecified(
    action: &str,
    kind: Option<&str>,
    transcript: &str,
) -> Option<(&'static str, &'static str, &'static str)> {
    // A named thing beats the app type: "skip to the next song" in a browser is media.
    let lower = transcript.to_lowercase();
    let names_media = ["song", "track", "music"].iter().any(|w| lower.contains(w));
    let names_tab = lower.contains("tab");
    let kind = if names_media {
        "music player"
    } else if names_tab {
        "browser"
    } else {
        kind.unwrap_or("other")
    };
    Some(match (action, kind) {
        ("go_back", "browser") if names_tab => ("shortcut", "name", "previous_tab"),
        ("go_back", "music player") => ("media", "control", "previous"),
        ("go_back", "browser") => ("shortcut", "name", "browser_back"),
        ("go_back", _) => ("shortcut", "name", "undo"),
        ("go_next", "music player") => ("media", "control", "next"),
        ("go_next", _) => ("shortcut", "name", "next_tab"),
        ("close_this", "browser" | "code editor") => ("shortcut", "name", "close_tab"),
        ("close_this", _) => ("shortcut", "name", "close_window"),
        // Tabs in tabbed apps; otherwise the window Jarvis tiled on that side.
        ("left_one", "browser" | "code editor") => ("shortcut", "name", "previous_tab"),
        ("left_one", _) => ("focus_side", "side", "left"),
        ("right_one", "browser" | "code editor") => ("shortcut", "name", "next_tab"),
        ("right_one", _) => ("focus_side", "side", "right"),
        _ => return None,
    })
}

/// d1 criterion text for a builtin enum value; unknown values read as their label.
fn option_hint(value: &str, context: Option<&str>) -> String {
    if context.is_some() {
        let focused_hint = match value {
            "undo" => Some("undo a change or go back one step in a browser or text editor"),
            "next_tab" => Some("move, go or switch to the next tab on the right in a browser or code editor"),
            "previous_tab" => Some("move, go or switch to the previous tab on the left in a browser or code editor"),
            "close_tab" => Some("close the current browser or code editor tab; close this in a browser"),
            "close_window" => Some("close the app window, including File Explorer or Notepad"),
            "previous" => Some("request the previous song/track, go back one track or step in a music player"),
            _ => None,
        };
        if let Some(hint) = focused_hint {
            return hint.to_string();
        }
    }
    match value {
        "cut" => "cut, cut that out",
        "undo" => "undo, take that back, go back one step, reverse my last edit, bring back what i deleted",
        "redo" => "redo, put the change back",
        "select_all" => "select all, highlight everything",
        "find" => "find a word on this page",
        "browser_back" => "go back to the previous page in the browser",
        "browser_forward" => "go forward to the next page in the browser",
        "reopen_tab" => "reopen or restore the tab just closed",
        "next_tab" => "next tab, the tab on the right",
        "previous_tab" => "previous tab, the tab on the left",
        "close_window" => "close this window or app",
        "switch_app" => "switch to another app, alt tab, jump to my other window",
        "show_desktop" => "show the desktop, clear everything off the screen, minimize everything",
        "minimize" => "minimize, hide or put away this window",
        "next" => "next or skip song/track",
        "previous" => "previous song/track, go back a track",
        "play_pause" => "play, pause, stop or resume the music",
        "mute" => "mute or silence all sound",
        "maximize" => "maximize, full screen",
        _ => return value.replace('_', " "),
    }
    .to_string()
}

fn unique_enum_values(values: &[String]) -> Vec<&str> {
    values
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn error(code: RouterErrorCode, message: impl Into<String>) -> RouterError {
    RouterError {
        code,
        message: message.into(),
    }
}

/// Build the combined tool and required-enum questions for one clause.
#[cfg(test)]
pub(crate) fn systemone_request(
    conn: &Connection,
    transcript: &str,
    tools: &[ToolDefinition],
) -> Result<Value, RouterError> {
    systemone_request_details(conn, transcript, None, tools, true)
}

fn systemone_request_details(
    conn: &Connection,
    transcript: &str,
    context: Option<&str>,
    tools: &[ToolDefinition],
    include_details: bool,
) -> Result<Value, RouterError> {
    let mut criteria = Map::new();
    let mut questions = Map::new();
    let mut enabled_count = 0usize;
    // Reached only through code: spoken key names, or "the left/right one". As d1
    // options they steal "next tab" and "switch windows".
    for tool in tools
        .iter()
        .filter(|tool| tool.enabled && !matches!(tool.name.as_str(), "press_keys" | "focus_side"))
    {
        if tool.parameters.iter().any(|param| {
            param.param_type == "enum"
                && param.required
                && !(1..=20).contains(&unique_enum_values(&param.enum_values).len())
        }) {
            continue;
        }
        if tool.name == "run_registered_script" {
            let mut stmt = conn
                .prepare("SELECT id, display_name FROM registered_scripts ORDER BY id")
                .map_err(|e| error(RouterErrorCode::SchemaInvalid, e.to_string()))?;
            let scripts = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| error(RouterErrorCode::SchemaInvalid, e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| error(RouterErrorCode::SchemaInvalid, e.to_string()))?;
            if scripts.is_empty() || scripts.len() > 20 {
                continue;
            }
            if include_details && scripts.len() > 1 {
                let mut script_criteria = Map::new();
                for (id, name) in scripts {
                    script_criteria.insert(id, json!(name));
                }
                questions.insert("run_registered_script__script_id".into(), json!({
                    "type": "choice", "instructions": "If running a saved script, which script is requested?",
                    "criteria": script_criteria
                }));
            }
        }
        criteria.insert(tool.name.clone(), json!(tool.description));
        enabled_count += 1;
        for param in tool
            .parameters
            .iter()
            .filter(|p| p.param_type == "enum" && p.required)
        {
            if !include_details {
                continue;
            }
            let values = unique_enum_values(&param.enum_values);
            if values.len() > 20 {
                continue;
            }
            if values.len() >= 2 {
                let options: Map<String, Value> = values
                    .iter()
                    .map(|v| ((*v).to_string(), json!(option_hint(v, context))))
                    .collect();
                questions.insert(format!("{}__{}", tool.name, param.name), json!({
                    "type": "choice", "instructions": format!("If this is a {} command, which {} is meant?", tool.display_name, param.name),
                    "criteria": options
                }));
            }
        }
    }
    if enabled_count + 1 > 20 || enabled_count == 0 {
        return Err(error(
            RouterErrorCode::SchemaInvalid,
            "systemone tool choice must have 2 to 20 options including NONE",
        ));
    }
    let kind = context.and_then(app_kind);
    if !include_details {
        if context.is_some() && criteria.len() + 1 + UNSPECIFIED.len() <= 20 {
            for (action, hint) in UNSPECIFIED {
                criteria.insert(action.into(), json!(hint));
            }
        }
        // Kept apart from the tool question so music talk can't win just by topic.
        questions.insert("instruction".into(), json!({
            "type": "choice", "instructions": "Is the speaker telling the computer to do something?",
            "criteria": {
                "instruction": "a request or command for the computer to do something",
                "remark": "a comment, opinion, feeling or memory that asks the computer for nothing"
            }
        }));
    }
    criteria.insert("NONE".into(), json!(NONE_HINT));
    questions.insert("tool".into(), json!({
        "type": "choice", "instructions": "Which action should the computer take for this voice command?",
        "criteria": criteria
    }));
    // State holds facts only; rules belong in criteria.
    let state = match (context, kind) {
        (Some(context), Some(kind)) => format!("Voice command to a desktop assistant while {context}, a {kind}: \"{transcript}\""),
        (Some(context), None) => format!("Voice command to a desktop assistant while {context}: \"{transcript}\""),
        (None, _) => format!("Voice command to a desktop assistant: \"{transcript}\""),
    };
    Ok(json!({ "state": state, "questions": questions }))
}

fn split_clauses(transcript: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < transcript.len() {
        let sep = [" and then ", " then ", " and "].iter().find(|s| {
            transcript
                .get(i..i + s.len())
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(s))
        });
        if let Some(sep) = sep {
            if parts.len() + 1 < MAX_ROUTER_TOOL_CALLS {
                let part = transcript[start..i].trim();
                if !part.is_empty() {
                    parts.push(part.to_string());
                }
                i += sep.len();
                start = i;
                continue;
            }
        }
        i += transcript[i..].chars().next().unwrap().len_utf8();
    }
    let tail = transcript[start..].trim();
    if !tail.is_empty() {
        parts.push(tail.to_string());
    }
    if parts.is_empty() {
        vec![transcript.trim().to_string()]
    } else {
        parts
    }
}

#[cfg(test)]
pub(crate) fn split_clauses_for_eval(transcript: &str) -> Vec<String> {
    split_clauses(transcript)
}

fn choice<'a>(response: &'a Value, id: &str) -> Result<(&'a str, f32), RouterError> {
    let answer = response
        .get("answers")
        .and_then(|v| v.get(id))
        .ok_or_else(|| {
            error(
                RouterErrorCode::InferFailed,
                format!("missing answer `{id}`"),
            )
        })?;
    let value = answer
        .get("choice")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            error(
                RouterErrorCode::InferFailed,
                format!("missing choice `{id}`"),
            )
        })?;
    let confidence = answer
        .get("confidence")
        .and_then(Value::as_f64)
        .ok_or_else(|| {
            error(
                RouterErrorCode::InferFailed,
                format!("missing confidence `{id}`"),
            )
        })?;
    if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
        return Err(error(
            RouterErrorCode::SchemaInvalid,
            format!("invalid confidence for `{id}`"),
        ));
    }
    Ok((value, confidence as f32))
}

fn decide_clause<F>(
    conn: &Connection,
    tools: &[ToolDefinition],
    transcript: &str,
    context: Option<&str>,
    threshold: f32,
    infer: &mut F,
) -> Result<RouterToolCallWithConfidence, RouterError>
where
    F: FnMut(&Value) -> Result<Value, String>,
{
    // A spoken key combo ("alt tab", "control z") is exact; no model needed.
    if let Some(keys) = crate::llm::decider_args::spoken_keys(transcript) {
        if tools.iter().any(|t| t.name == "press_keys" && t.enabled) {
            return Ok(RouterToolCallWithConfidence {
                call: RouterToolCall {
                    tool: "press_keys".into(),
                    args: [("keys".to_string(), keys)].into(),
                },
                confidence: 1.0,
            });
        }
    }
    let body = systemone_request_details(conn, transcript, context, tools, false)?;
    let response = infer(&body).map_err(|e| error(RouterErrorCode::InferFailed, e))?;
    let (name, confidence) = choice(&response, "tool")?;
    // "close spotify" names an app: closing the focused window would close the wrong thing.
    let name = if matches!(name, "shortcut" | "close_this")
        && extract_args("close_app", transcript).is_some()
        && tools.iter().any(|t| t.name == "close_app" && t.enabled)
    {
        "close_app".to_string()
    } else {
        name.to_string()
    };
    // Free-text tools already require an explicit verb ("say ..."), so only the
    // verb-less tools need the remark check.
    let verb_guarded = matches!(
        name.as_str(),
        "open_target" | "open_url" | "web_search" | "type_text" | "say" | "close_app"
    );
    if !verb_guarded && response["answers"].get("instruction").is_some() {
        let (kind, instruction_confidence) = choice(&response, "instruction")?;
        if kind == "remark" && instruction_confidence >= threshold {
            return Err(error(RouterErrorCode::LowConfidence, "d1 heard a remark, not an instruction"));
        }
    }
    if name == "NONE" || confidence < threshold {
        return Err(error(
            RouterErrorCode::LowConfidence,
            if name == "NONE" {
                "d1 selected NONE"
            } else {
                "d1 confidence below threshold"
            },
        ));
    }
    let mut result = finish_clause(
        conn, tools, transcript, context, threshold, infer, &body, &response, &name, confidence,
    );
    // A pick with no usable args ("paste it" as type_text) falls through to the next
    // option, scored as its probability given every rejected pick was wrong.
    let mut rejected = vec![name];
    let mut rejected_mass = f64::from(confidence);
    loop {
        match &result {
            Err(err)
                if err.code == RouterErrorCode::LowConfidence
                    && err.message.starts_with("could not extract") => {}
            _ => return result,
        }
        let Some((next, p)) = response["answers"]["tool"]["probabilities"]
            .as_object()
            .and_then(|probs| {
                probs
                    .iter()
                    .filter(|(option, _)| !rejected.contains(option))
                    .filter_map(|(option, p)| Some((option.clone(), p.as_f64()?)))
                    .max_by(|a, b| a.1.total_cmp(&b.1))
            })
        else {
            return result;
        };
        if next == "NONE" || rejected_mass >= 1.0 {
            return result;
        }
        let conditional = (p / (1.0 - rejected_mass)) as f32;
        if conditional < threshold {
            return result;
        }
        rejected_mass += p;
        result = finish_clause(
            conn, tools, transcript, context, threshold, infer, &body, &response, &next, conditional,
        );
        rejected.push(next);
    }
}

#[allow(clippy::too_many_arguments)]
fn finish_clause<F>(
    conn: &Connection,
    tools: &[ToolDefinition],
    transcript: &str,
    context: Option<&str>,
    threshold: f32,
    infer: &mut F,
    body: &Value,
    response: &Value,
    name: &str,
    confidence: f32,
) -> Result<RouterToolCallWithConfidence, RouterError>
where
    F: FnMut(&Value) -> Result<Value, String>,
{
    if let Some((tool_name, key, value)) = context
        .and_then(|context| bind_unspecified(name, app_kind(context), transcript))
    {
        let tool = tools
            .iter()
            .find(|t| t.name == tool_name && t.enabled)
            .ok_or_else(|| {
                error(
                    RouterErrorCode::ToolDisabled,
                    format!("`{name}` needs disabled tool `{tool_name}`"),
                )
            })?;
        let args = Map::from_iter([(key.to_string(), json!(value))]);
        let checked = super::router::validate_args_against_tool(tool, &args)?;
        return Ok(RouterToolCallWithConfidence {
            call: RouterToolCall {
                tool: tool_name.to_string(),
                args: checked,
            },
            confidence,
        });
    }
    // "left"/"right" lure d1 toward snapping; a named tab or an open verb means
    // something else, so let the next choice take it.
    if name == "snap_window"
        && (transcript
            .to_lowercase()
            .split_whitespace()
            .any(|w| w == "tab" || w == "tabs")
            || crate::llm::decider_args::starts_with_open_verb(transcript))
    {
        return Err(error(
            RouterErrorCode::LowConfidence,
            "could not extract a window to snap: the command names a tab or an app",
        ));
    }
    // d1 often reads a spoken address ("youtube dot com") as an app name.
    let name = if name == "open_target"
        && crate::llm::decider_args::first_explicit_url(transcript).is_some()
        && tools.iter().any(|t| t.name == "open_url" && t.enabled)
    {
        "open_url".to_string()
    } else {
        name.to_string()
    };
    let tool = tools
        .iter()
        .find(|t| t.name == name && t.enabled)
        .ok_or_else(|| {
            error(
                RouterErrorCode::UnknownTool,
                format!("d1 selected unavailable tool `{name}`"),
            )
        })?;
    if body["questions"]["tool"]["criteria"].get(&name).is_none() {
        return Err(error(
            RouterErrorCode::ToolDisabled,
            format!("d1 selected excluded tool `{name}`"),
        ));
    }
    let response = {
        let detail_body =
            systemone_request_details(conn, transcript, context, std::slice::from_ref(tool), true)?;
        let mut detail_body = detail_body;
        if let Some(questions) = detail_body
            .get_mut("questions")
            .and_then(Value::as_object_mut)
        {
            questions.remove("tool");
        }
        if detail_body["questions"]
            .as_object()
            .is_some_and(|questions| !questions.is_empty())
        {
            infer(&detail_body).map_err(|e| error(RouterErrorCode::InferFailed, e))?
        } else {
            response.clone()
        }
    };
    let mut args = if let Some(args) = extract_args(&name, transcript) {
        args
    } else {
        Map::new()
    };
    if name == "run_registered_script" {
        let script_id = if response
            .get("answers")
            .and_then(|v| v.get("run_registered_script__script_id"))
            .is_some()
        {
            choice(&response, "run_registered_script__script_id")?
                .0
                .to_string()
        } else {
            let mut stmt = conn
                .prepare("SELECT id FROM registered_scripts ORDER BY id")
                .map_err(|e| error(RouterErrorCode::SchemaInvalid, e.to_string()))?;
            let mut ids = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| error(RouterErrorCode::SchemaInvalid, e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| error(RouterErrorCode::SchemaInvalid, e.to_string()))?;
            if ids.len() != 1 {
                return Err(error(
                    RouterErrorCode::SchemaInvalid,
                    "script choice missing",
                ));
            }
            ids.remove(0)
        };
        let script = get_registered_script(conn, &script_id)
            .map_err(|e| error(RouterErrorCode::SchemaInvalid, e.to_string()))?
            .ok_or_else(|| {
                error(
                    RouterErrorCode::SchemaInvalid,
                    "d1 selected an unknown registered script",
                )
            })?;
        // Scripts can do anything, so only run one the user actually named.
        if !names_script(transcript, &script.display_name, &script.id) {
            return Err(error(
                RouterErrorCode::LowConfidence,
                format!("`{}` wasn't named, so it won't run", script.display_name),
            ));
        }
        args.insert("script_id".into(), json!(script_id));
    }
    for param in tool
        .parameters
        .iter()
        .filter(|p| p.param_type == "enum" && p.required)
    {
        let values = unique_enum_values(&param.enum_values);
        if values.len() == 1 {
            args.insert(param.name.clone(), json!(values[0]));
        } else if (2..=20).contains(&values.len()) {
            let id = format!("{name}__{}", param.name);
            let (value, enum_confidence) = choice(&response, &id)?;
            if enum_confidence < threshold {
                return Err(error(
                    RouterErrorCode::LowConfidence,
                    format!("d1 confidence below threshold for `{id}`"),
                ));
            }
            if !values.contains(&value) {
                return Err(error(
                    RouterErrorCode::SchemaInvalid,
                    format!("d1 selected invalid enum for `{id}`"),
                ));
            }
            args.insert(param.name.clone(), json!(value));
        }
    }
    if extract_args(&name, transcript).is_none()
        && matches!(
            name.as_str(),
            "open_target" | "open_url" | "web_search" | "type_text" | "say"
        )
    {
        return Err(error(
            RouterErrorCode::LowConfidence,
            format!("could not extract required arguments for `{name}`"),
        ));
    }
    if let Some(param) = tool.parameters.iter().find(|param| {
        param.required
            && args
                .get(&param.name)
                .is_none_or(|value| value.as_str().is_none_or(|value| value.trim().is_empty()))
    }) {
        return Err(error(
            RouterErrorCode::LowConfidence,
            format!(
                "could not extract required argument `{}` for `{name}`",
                param.name
            ),
        ));
    }
    let mut call = RouterToolCall {
        tool: name,
        args: args
            .into_iter()
            .map(|(key, value)| {
                value
                    .as_str()
                    .map(|value| (key, value.to_string()))
                    .ok_or_else(|| {
                        error(
                            RouterErrorCode::SchemaInvalid,
                            "router argument must be a string",
                        )
                    })
            })
            .collect::<Result<_, _>>()?,
    };
    if call.tool == "open_url" {
        call = crate::llm::intent_validate::normalize_router_tool_calls(vec![call], &[], &[])?
            .pop()
            .expect("one open_url call normalizes to one call");
    }
    let final_tool = tools
        .iter()
        .find(|candidate| candidate.name == call.tool && candidate.enabled)
        .ok_or_else(|| {
            error(
                RouterErrorCode::ToolDisabled,
                format!("normalized tool `{}` is unavailable", call.tool),
            )
        })?;
    let normalized_args: Map<String, Value> = call
        .args
        .iter()
        .map(|(key, value)| (key.clone(), json!(value)))
        .collect();
    let checked = super::router::validate_args_against_tool(final_tool, &normalized_args)?;
    Ok(RouterToolCallWithConfidence {
        call: RouterToolCall {
            tool: call.tool,
            args: checked,
        },
        confidence,
    })
}

struct RouterToolCallWithConfidence {
    call: RouterToolCall,
    confidence: f32,
}

/// Decide clauses; split sequences are accepted only when every clause is safe.
pub fn decide(
    conn: &Connection,
    tools: &[ToolDefinition],
    transcript: &str,
    context: Option<&str>,
    threshold: f32,
    port: u16,
) -> Result<RouterRouteResult, RouterError> {
    decide_with_infer(conn, tools, transcript, context, threshold, |body| {
        super::local::systemone(port, body)
    })
}

#[cfg(test)]
pub(crate) fn decide_eval(
    conn: &Connection,
    tools: &[ToolDefinition],
    transcript: &str,
    context: Option<&str>,
    threshold: f32,
    port: u16,
) -> Result<RouterRouteResult, RouterError> {
    decide_with_infer(conn, tools, transcript, context, threshold, |body| {
        super::local::systemone(port, body)
    })
}

fn decide_with_infer<F>(
    conn: &Connection,
    tools: &[ToolDefinition],
    transcript: &str,
    context: Option<&str>,
    threshold: f32,
    mut infer: F,
) -> Result<RouterRouteResult, RouterError>
where
    F: FnMut(&Value) -> Result<Value, String>,
{
    if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
        return Err(error(
            RouterErrorCode::SchemaInvalid,
            "router confidence threshold must be from 0 to 1",
        ));
    }
    let parts = split_clauses(transcript);
    let mut calls = Vec::new();
    let mut confidence = 1.0f32;
    if parts.len() > 1 {
        let split_result: Result<(), RouterError> = parts.iter().try_for_each(|part| {
            let result = decide_clause(conn, tools, part, context, threshold, &mut infer)?;
            calls.push(result.call);
            confidence = confidence.min(result.confidence);
            Ok(())
        });
        if split_result.is_ok() {
            return Ok(RouterRouteResult {
                tool_calls: calls,
                confidence,
            });
        }
    }
    let result = decide_clause(conn, tools, transcript, context, threshold, &mut infer)?;
    Ok(RouterRouteResult {
        tool_calls: vec![result.call],
        confidence: result.confidence,
    })
}

#[cfg(test)]
fn decide_with<F>(
    conn: &Connection,
    tools: &[ToolDefinition],
    transcript: &str,
    threshold: f32,
    infer: F,
) -> Result<RouterRouteResult, RouterError>
where
    F: FnMut(&Value) -> Result<Value, String>,
{
    decide_with_infer(conn, tools, transcript, None, threshold, infer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{get_all_tools, init_db};

    fn db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        init_db(&path).unwrap();
        (dir, Connection::open(path).unwrap())
    }

    #[test]
    fn builds_tool_and_enum_questions_with_none() {
        let (_dir, conn) = db();
        let tools = get_all_tools(&conn).unwrap();
        let body = systemone_request(&conn, "pause the music", &tools).unwrap();
        assert_eq!(body["questions"]["tool"]["criteria"]["NONE"], NONE_HINT);
        assert!(body["questions"].get("media__control").is_some());
        assert_eq!(
            body["questions"]["tool"]["criteria"]["say"],
            tools
                .iter()
                .find(|tool| tool.name == "say")
                .unwrap()
                .description
        );
    }

    #[test]
    fn snapping_yields_to_a_named_tab() {
        let (_dir, conn) = db();
        let tools = get_all_tools(&conn).unwrap();
        let route = decide_with(&conn, &tools, "go to the tab on the left", 0.5, |body| {
            if body["questions"].get("tool").is_some() {
                Ok(json!({"answers":{"tool":{"choice":"snap_window","confidence":0.6,
                    "probabilities":{"snap_window":0.6,"shortcut":0.38,"NONE":0.02}}}}))
            } else {
                Ok(json!({"answers":{"shortcut__name":{"choice":"previous_tab","confidence":0.9}}}))
            }
        })
        .unwrap();
        assert_eq!(route.tool_calls[0].args["name"], "previous_tab");
    }

    #[test]
    fn scripts_run_only_when_named() {
        assert!(names_script("run my morning routine", "Morning Routine", "morning_routine"));
        assert!(names_script("start the morning routin script", "Morning Routine", "x"));
        assert!(!names_script("do that again", "Morning Routine", "morning_routine"));
        assert!(!names_script("run the routine", "Morning Routine", "morning_routine"));
    }

    #[test]
    fn app_kind_reads_only_the_app_name() {
        assert_eq!(app_kind("the user is in Visual Studio Code (window: \"a.rs\")"), Some("code editor"));
        assert_eq!(app_kind("the user is in Google Chrome (window: \"Spotify\")"), Some("browser"));
        assert_eq!(app_kind("the user is in Spotify"), Some("music player"));
        assert_eq!(app_kind("the user is in Microsoft Excel"), None);
    }

    #[test]
    fn unspecified_actions_bind_by_app_kind_and_remarks_are_rejected() {
        let (_dir, conn) = db();
        let tools = get_all_tools(&conn).unwrap();
        let route = |context: &str, pick: &str| {
            decide_with_infer(&conn, &tools, "go back", Some(context), 0.5, |body| {
                assert!(body["questions"]["tool"]["criteria"].get(pick).is_some());
                Ok(json!({"answers":{"tool":{"choice":pick,"confidence":0.9},
                    "instruction":{"choice":"instruction","confidence":0.9}}}))
            })
            .unwrap()
            .tool_calls
            .remove(0)
        };
        let back = route("the user is in Spotify", "go_back");
        assert_eq!((back.tool.as_str(), back.args["control"].as_str()), ("media", "previous"));
        let close = route("the user is in Google Chrome", "close_this");
        assert_eq!(close.args["name"], "close_tab");
        let close = route("the user is in File Explorer", "close_this");
        assert_eq!(close.args["name"], "close_window");
        assert_eq!(bind_unspecified("go_next", Some("browser"), "skip to the next song"), Some(("media", "control", "next")));
        assert_eq!(bind_unspecified("go_back", Some("music player"), "go back a tab"), Some(("shortcut", "name", "previous_tab")));

        let remark = decide_with_infer(&conn, &tools, "this music takes me back", Some("the user is in Spotify"), 0.5, |_| {
            Ok(json!({"answers":{"tool":{"choice":"media","confidence":0.9},
                "instruction":{"choice":"remark","confidence":0.8}}}))
        })
        .unwrap_err();
        assert_eq!(remark.code, RouterErrorCode::LowConfidence);

        // Without context the app-dependent options aren't offered.
        decide_with(&conn, &tools, "go back", 0.5, |body| {
            assert!(body["questions"]["tool"]["criteria"].get("go_back").is_none());
            Ok(json!({"answers":{"tool":{"choice":"NONE","confidence":0.9}}}))
        })
        .unwrap_err();
    }

    #[test]
    fn context_reaches_both_calls_for_every_clause() {
        let (_dir, conn) = db();
        let tools = get_all_tools(&conn).unwrap();
        let mut states = Vec::new();
        let context = "the user is in Spotify";
        let result = decide_with_infer(
            &conn,
            &tools,
            "pause music and then skip song",
            Some(context),
            0.5,
            |body| {
                let state = body["state"].as_str().unwrap();
                assert!(state.contains(context));
                states.push(state.to_string());
                if body["questions"].get("tool").is_some() {
                    Ok(json!({"answers":{"tool":{"choice":"media","confidence":0.99}}}))
                } else {
                    Ok(json!({"answers":{"media__control":{"choice":"next","confidence":0.99}}}))
                }
            },
        )
        .unwrap();
        assert_eq!(result.tool_calls.len(), 2);
        assert_eq!(states.len(), 4);
        assert_eq!(states[0], states[1]);
        assert_eq!(states[2], states[3]);
        assert!(states[0].contains("\"pause music\""));
        assert!(states[2].contains("\"skip song\""));
    }

    #[test]
    fn no_context_preserves_state_text_in_both_calls() {
        let (_dir, conn) = db();
        let tools = get_all_tools(&conn).unwrap();
        let mut count = 0;
        decide_with_infer(&conn, &tools, "pause music", None, 0.5, |body| {
            count += 1;
            assert_eq!(body["state"], "Voice command to a desktop assistant: \"pause music\"");
            Ok(json!({"answers":{
                "tool":{"choice":"media","confidence":0.99},
                "media__control":{"choice":"play_pause","confidence":0.99}
            }}))
        })
        .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn splits_only_up_to_three_clauses() {
        assert_eq!(
            split_clauses("open mail and then search cats then say hi"),
            ["open mail", "search cats", "say hi"]
        );
        assert_eq!(
            split_clauses("search salt and pepper"),
            ["search salt", "pepper"]
        );
        assert_eq!(split_clauses("café and then say hi"), ["café", "say hi"]);
    }

    #[test]
    fn parses_canned_systemone_choice_and_rejects_bad_confidence() {
        let good = json!({"answers":{"tool":{"choice":"media","confidence":0.91}}});
        assert_eq!(choice(&good, "tool").unwrap(), ("media", 0.91));
        let bad = json!({"answers":{"tool":{"choice":"media","confidence":1.7}}});
        assert_eq!(
            choice(&bad, "tool").unwrap_err().code,
            RouterErrorCode::SchemaInvalid
        );
    }

    #[test]
    fn decide_accepts_thresholded_free_text_and_rejects_none_or_low_confidence() {
        let (_dir, conn) = db();
        let tools = get_all_tools(&conn).unwrap();
        let accepted = decide_with(&conn, &tools, "say hello", 0.7, |_| {
            Ok(json!({"answers":{"tool":{"choice":"say","confidence":0.9}}}))
        })
        .unwrap();
        assert_eq!(accepted.tool_calls[0].tool, "say");
        assert_eq!(
            accepted.tool_calls[0].args.get("text").map(String::as_str),
            Some("hello")
        );
        let none = decide_with(&conn, &tools, "random noise", 0.7, |_| {
            Ok(json!({"answers":{"tool":{"choice":"NONE","confidence":0.99}}}))
        })
        .unwrap_err();
        assert_eq!(none.code, RouterErrorCode::LowConfidence);
        let low = decide_with(&conn, &tools, "say hello", 0.7, |_| {
            Ok(json!({"answers":{"tool":{"choice":"say","confidence":0.4}}}))
        })
        .unwrap_err();
        assert_eq!(low.code, RouterErrorCode::LowConfidence);
    }

    #[test]
    fn decide_checks_enum_ids_and_retries_unsplit_after_a_split_failure() {
        let (_dir, conn) = db();
        let tools = get_all_tools(&conn).unwrap();
        let mut calls = 0;
        let result = decide_with(&conn, &tools, "say hello and then say goodbye", 0.7, |_| {
            calls += 1;
            Ok(if calls == 1 {
                json!({"answers":{"tool":{"choice":"NONE","confidence":0.99}}})
            } else {
                json!({"answers":{"tool":{"choice":"say","confidence":0.9}}})
            })
        })
        .unwrap();
        assert_eq!(result.tool_calls.len(), 1);
        let invalid = decide_with(&conn, &tools, "pause the music", 0.7, |_| Ok(json!({"answers":{"tool":{"choice":"media","confidence":0.9},"media__control":{"choice":"wrong","confidence":0.9}}}))).unwrap_err();
        assert_eq!(invalid.code, RouterErrorCode::SchemaInvalid);
    }

    #[test]
    fn falls_back_to_runner_up_when_top_pick_has_no_args() {
        let (_dir, conn) = db();
        let tools = get_all_tools(&conn).unwrap();
        let tool = json!({"answers":{"tool":{"choice":"type_text","confidence":0.6,
            "probabilities":{"type_text":0.6,"shortcut":0.38,"NONE":0.02}}}});
        let route = decide_with(&conn, &tools, "paste it", 0.5, |body| {
            if body["questions"].get("tool").is_some() {
                Ok(tool.clone())
            } else {
                Ok(json!({"answers":{"shortcut__name":{"choice":"paste","confidence":0.9}}}))
            }
        })
        .unwrap();
        assert_eq!(route.tool_calls[0].tool, "shortcut");
        // 0.6 top, 0.1 runner-up: 0.1 / 0.4 = 0.25 stays below the threshold.
        let weak = json!({"answers":{"tool":{"choice":"type_text","confidence":0.6,
            "probabilities":{"type_text":0.6,"shortcut":0.1,"NONE":0.3}}}});
        let err = decide_with(&conn, &tools, "paste it", 0.5, |_| Ok(weak.clone())).unwrap_err();
        assert_eq!(err.code, RouterErrorCode::LowConfidence);
    }

    #[test]
    fn two_call_variant_asks_enum_only_after_tool_choice() {
        let (_dir, conn) = db();
        let tools = get_all_tools(&conn).unwrap();
        let mut calls = 0;
        let route = decide_with(&conn, &tools, "open a new tab", 0.7, |body| {
            calls += 1;
            if body["questions"].get("tool").is_some() {
                Ok(json!({"answers":{"tool":{"choice":"shortcut","confidence":0.9}}}))
            } else {
                Ok(json!({"answers":{"shortcut__name":{"choice":"new_tab","confidence":0.9}}}))
            }
        })
        .unwrap();
        let low_enum = decide_with(&conn, &tools, "open a new tab", 0.7, |body| {
            if body["questions"].get("tool").is_some() {
                Ok(json!({"answers":{"tool":{"choice":"shortcut","confidence":0.9}}}))
            } else {
                Ok(json!({"answers":{"shortcut__name":{"choice":"new_tab","confidence":0.2}}}))
            }
        })
        .unwrap_err();
        assert_eq!(low_enum.code, RouterErrorCode::LowConfidence);
        assert_eq!(calls, 2);
        assert_eq!(
            route.tool_calls[0].args.get("name").map(String::as_str),
            Some("new_tab")
        );
        assert_eq!(route.confidence, 0.9);
    }

    #[test]
    fn singleton_registered_script_is_direct_and_validated() {
        let (_dir, conn) = db();
        crate::db::upsert_registered_script(
            &conn,
            &crate::db::RegisteredScript {
                id: "morning_routine".into(),
                path: "fixture".into(),
                hash: "fixture".into(),
                display_name: "Morning routine".into(),
                arg_names: Vec::new(),
            },
        )
        .unwrap();
        let tools = get_all_tools(&conn).unwrap();
        let body = systemone_request(&conn, "run my morning routine", &tools).unwrap();
        assert!(body["questions"]
            .get("run_registered_script__script_id")
            .is_none());
        let route = decide_with(&conn, &tools, "run my morning routine", 0.7, |_| {
            Ok(json!({"answers":{"tool":{"choice":"run_registered_script","confidence":0.9}}}))
        })
        .unwrap();
        assert_eq!(
            route.tool_calls[0]
                .args
                .get("script_id")
                .map(String::as_str),
            Some("morning_routine")
        );
    }

    #[test]
    fn script_choice_supports_twenty_and_excludes_more_than_twenty() {
        let (_dir, conn) = db();
        for n in 0..20 {
            crate::db::upsert_registered_script(
                &conn,
                &crate::db::RegisteredScript {
                    id: format!("script-{n:02}"),
                    path: "fixture".into(),
                    hash: "fixture".into(),
                    display_name: format!("Script {n}"),
                    arg_names: Vec::new(),
                },
            )
            .unwrap();
        }
        let tools = get_all_tools(&conn).unwrap();
        let body = systemone_request(&conn, "run one", &tools).unwrap();
        assert_eq!(
            body["questions"]["run_registered_script__script_id"]["criteria"]
                .as_object()
                .unwrap()
                .len(),
            20
        );
        crate::db::upsert_registered_script(
            &conn,
            &crate::db::RegisteredScript {
                id: "script-20".into(),
                path: "fixture".into(),
                hash: "fixture".into(),
                display_name: "Script 20".into(),
                arg_names: Vec::new(),
            },
        )
        .unwrap();
        let body = systemone_request(&conn, "run one", &tools).unwrap();
        assert!(body["questions"]["tool"]["criteria"]
            .get("run_registered_script")
            .is_none());
        let error = decide_with(&conn, &tools, "run one", 0.7, |_| {
            Ok(json!({"answers":{"tool":{"choice":"run_registered_script","confidence":0.99}}}))
        })
        .unwrap_err();
        assert_eq!(error.code, RouterErrorCode::ToolDisabled);
    }

    #[test]
    fn rejects_out_of_range_threshold_before_inference() {
        let (_dir, conn) = db();
        let tools = get_all_tools(&conn).unwrap();
        let error = decide_with(&conn, &tools, "say hi", f32::NAN, |_| {
            panic!("must not infer")
        })
        .unwrap_err();
        assert_eq!(error.code, RouterErrorCode::SchemaInvalid);
    }

    #[test]
    fn bare_open_url_falls_back_to_open_target_with_placement() {
        let (_dir, conn) = db();
        let tools = get_all_tools(&conn).unwrap();
        let route = decide_with(&conn, &tools, "open spotify on the left", 0.7, |_| {
            Ok(json!({"answers":{"tool":{"choice":"open_url","confidence":0.9}}}))
        })
        .unwrap();
        assert_eq!(route.tool_calls[0].tool, "open_target");
        assert_eq!(
            route.tool_calls[0].args.get("target").map(String::as_str),
            Some("spotify")
        );
        assert_eq!(
            route.tool_calls[0]
                .args
                .get("placement")
                .map(String::as_str),
            Some("left_half")
        );
    }

    #[test]
    fn bare_open_url_fallback_rejects_disabled_open_target() {
        let (_dir, conn) = db();
        let mut tools = get_all_tools(&conn).unwrap();
        tools
            .iter_mut()
            .find(|tool| tool.name == "open_target")
            .unwrap()
            .enabled = false;
        let error = decide_with(&conn, &tools, "open spotify", 0.7, |_| {
            Ok(json!({"answers":{"tool":{"choice":"open_url","confidence":0.9}}}))
        })
        .unwrap_err();
        assert_eq!(error.code, RouterErrorCode::ToolDisabled);
    }

    #[test]
    fn duplicate_enum_singleton_is_direct_and_skips_second_call() {
        let (_dir, conn) = db();
        let mut tools = get_all_tools(&conn).unwrap();
        let shortcut = tools
            .iter_mut()
            .find(|tool| tool.name == "shortcut")
            .unwrap();
        shortcut.parameters[0].enum_values = vec!["undo".into(), "undo".into()];
        let body = systemone_request(&conn, "undo", &tools).unwrap();
        assert!(body["questions"].get("shortcut__name").is_none());
        let mut calls = 0;
        let route = decide_with(&conn, &tools, "undo", 0.7, |body| {
            calls += 1;
            assert!(body["questions"].get("shortcut__name").is_none());
            Ok(json!({"answers":{"tool":{"choice":"shortcut","confidence":0.9}}}))
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(
            route.tool_calls[0].args.get("name").map(String::as_str),
            Some("undo")
        );
    }

    #[test]
    fn enum_question_boundaries_count_distinct_values() {
        let (_dir, conn) = db();
        let mut tools = get_all_tools(&conn).unwrap();
        let mut shortcut = tools
            .iter_mut()
            .find(|tool| tool.name == "shortcut")
            .unwrap();
        shortcut.parameters[0].enum_values = (0..20)
            .map(|n| format!("value_{n}"))
            .chain(["value_0".to_string()])
            .collect();
        let body = systemone_request(&conn, "do it", &tools).unwrap();
        assert_eq!(
            body["questions"]["shortcut__name"]["criteria"]
                .as_object()
                .unwrap()
                .len(),
            20
        );
        shortcut = tools
            .iter_mut()
            .find(|tool| tool.name == "shortcut")
            .unwrap();
        shortcut.parameters[0].enum_values.push("value_20".into());
        let body = systemone_request(&conn, "do it", &tools).unwrap();
        assert!(body["questions"]["tool"]["criteria"]
            .get("shortcut")
            .is_none());
    }
}
