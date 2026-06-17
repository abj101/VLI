//! Tiered command routing: pinned trigger match → on-device LLM router → tool clarify.

use crate::{
    audio::{self, SharedAudioPipeline},
    AppIndexStore,
    cancel_active_run_in_state,
    commands::{
        executor::{ActionRuntime, TauriActionRuntime},
        execute_command_with_context, execute_tool, match_command, matcher::MatchResult,
        open_target::execute_open_target_with_aliases,
        ToolCallContext,
    },
    apps::intent::{classify_open_target, parse_open_intent, OpenIntent},
    db::{self, get_all_tools, get_app_settings, get_all_target_aliases, CommandNode},
    finalize_command_run,
    hide_hud_window,
    hud::HudPhase,
    llm::{
        infer_router_completion, llm_compile_backend, llm_gpu_runtime_available,
        normalize_router_tool_calls, resolve_router_model_path, route_transcript_with_infer,
        RouterError, RouterErrorCode, RouterInfer, RouterRouteResult,
    },
    load_default_fuzzy_threshold_pct,
    open_db_connection,
    read_command_cache,
    resolve_fuzzy_threshold_pct,
    set_phase,
    should_finalize_execution,
    await_follow_up_input,
    CommandCache, SharedHud, ACTION_RUN_CANCELLED_MSG,
};
use log::{debug, info, warn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

struct LocalInfer {
    model_path: String,
    use_gpu: bool,
}

impl RouterInfer for LocalInfer {
    fn infer(&self, prompt: &str) -> Result<String, String> {
        infer_router_completion(&self.model_path, prompt, self.use_gpu)
    }
}

fn matcher_nodes(
    nodes: &[CommandNode],
    default_threshold_pct: u16,
) -> Vec<CommandNode> {
    nodes
        .iter()
        .cloned()
        .map(|mut node| {
            node.fuzzy_threshold_pct =
                resolve_fuzzy_threshold_pct(node.fuzzy_threshold_pct, default_threshold_pct);
            node
        })
        .collect()
}

fn listening_gate(rt: &SharedHud) -> Result<(), String> {
    let s = rt.lock().map_err(|_| "hud state poisoned".to_string())?;
    if s.phase != HudPhase::Listening || !s.visible {
        debug!("flow: skip route (phase changed before commit)");
        return Err("skip".into());
    }
    Ok(())
}

fn route_commit_gate(rt: &SharedHud) -> Result<(), String> {
    let s = rt.lock().map_err(|_| "hud state poisoned".to_string())?;
    if !s.visible {
        debug!("flow: skip route (hud not visible)");
        return Err("skip".into());
    }
    if !matches!(s.phase, HudPhase::Listening | HudPhase::Routing) {
        debug!("flow: skip route (phase changed before commit)");
        return Err("skip".into());
    }
    Ok(())
}

fn spawn_action_run<F>(app: &AppHandle, rt: &SharedHud, audio: &SharedAudioPipeline, run: F)
where
    F: FnOnce() + Send + 'static,
{
    let app_h = app.clone();
    let rt_h = Arc::clone(rt);
    let audio_h = audio.clone();
    std::thread::spawn(move || {
        run();
        let should_finalize = {
            let mut s = match rt_h.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            let session_id = s.active_run_session_id.unwrap_or(s.session_id);
            let cancelled = s
                .active_run_cancel
                .as_ref()
                .is_some_and(|f| f.load(Ordering::Relaxed));
            let allowed = should_finalize_execution(&s, session_id, cancelled);
            if allowed {
                s.active_run_cancel = None;
                s.active_run_session_id = None;
            }
            allowed
        };
        if !should_finalize {
            return;
        }
        debug!("flow: command run complete; dismissing hud");
        finalize_command_run(&app_h, &rt_h, &audio_h);
    });
}

fn begin_execution_session(
    app: &AppHandle,
    rt: &SharedHud,
    audio: &SharedAudioPipeline,
    from_routing: bool,
) -> Result<(u64, Arc<AtomicBool>), String> {
    if from_routing {
        route_commit_gate(rt)?;
    } else {
        let s = rt.lock().map_err(|_| "hud state poisoned".to_string())?;
        if !s.visible || !matches!(s.phase, HudPhase::Listening | HudPhase::Matched) {
            debug!("flow: skip execute (phase changed before commit)");
            return Err("skip".into());
        }
    }
    debug!("flow: stopping mic pipeline");
    audio::stop_shared_pipeline(app, audio);
    let executing_session_id = set_phase(app, rt, HudPhase::Executing)?;
    hide_hud_window(app);
    let cancel_flag = Arc::new(AtomicBool::new(false));
    {
        let mut s = rt.lock().map_err(|_| "hud state poisoned".to_string())?;
        if s.session_id != executing_session_id || s.phase != HudPhase::Executing {
            debug!("flow: skip execute spawn (phase/session changed)");
            return Err("skip".into());
        }
        cancel_active_run_in_state(&mut s);
        s.active_run_cancel = Some(cancel_flag.clone());
        s.active_run_session_id = Some(executing_session_id);
        s.pending_follow_up_response = None;
        s.pending_follow_up_candidate = None;
        s.pending_follow_up_candidate_at = None;
    }
    Ok((executing_session_id, cancel_flag))
}

fn follow_up_runtime<'a>(
    app: &'a AppHandle,
    rt: &'a SharedHud,
    audio: &'a SharedAudioPipeline,
    executing_session_id: u64,
    cancel_flag: Arc<AtomicBool>,
) -> TauriActionRuntime<'a> {
    let app_for_followup = app.clone();
    let rt_for_followup = Arc::clone(rt);
    let audio_for_followup = audio.clone();
    let followup_cancel = cancel_flag.clone();
    TauriActionRuntime::with_follow_up_handler(
        app,
        cancel_flag,
        Box::new(move |prompt| {
            let response = await_follow_up_input(
                &app_for_followup,
                &rt_for_followup,
                &audio_for_followup,
                executing_session_id,
                &followup_cancel,
                prompt,
            )?;
            audio::stop_shared_pipeline(&app_for_followup, &audio_for_followup);
            let _ = set_phase(&app_for_followup, &rt_for_followup, HudPhase::Executing)?;
            Ok(response)
        }),
    )
}

fn execute_tier1_match(
    app: &AppHandle,
    rt: &SharedHud,
    audio: &SharedAudioPipeline,
    nodes: &[CommandNode],
    matched: MatchResult,
) -> Result<(), String> {
    listening_gate(rt)?;
    info!(
        "flow: MATCH node_id={} phrase={:?} span={}..{}",
        matched.node_id, matched.matched_phrase, matched.span_start, matched.span_end
    );
    let _ = app.emit("match-result", &matched);
    let _ = set_phase(app, rt, HudPhase::Matched)?;

    let (executing_session_id, cancel_flag) = begin_execution_session(app, rt, audio, false)?;
    let Some(node) = nodes.iter().find(|n| n.id.to_string() == matched.node_id) else {
        warn!(
            "flow: matched node_id={} but no row in loaded nodes (count={})",
            matched.node_id,
            nodes.len()
        );
        finalize_command_run(app, rt, audio);
        return Ok(());
    };
    let node = node.clone();
    info!("flow: spawn execute_command for node_id={}", node.id);
    let tool_context = if matched.remainder.is_empty() {
        None
    } else {
        Some(ToolCallContext::with_remainder(&matched.remainder))
    };
    let app_index_snapshot = {
        let st = app.state::<AppIndexStore>();
        let guard = st
            .read()
            .map_err(|_| "app index lock poisoned".to_string())?;
        guard.clone()
    };
    let target_aliases_snapshot = open_db_connection(app)
        .ok()
        .and_then(|conn| db::get_all_target_aliases(&conn).ok())
        .unwrap_or_default();
    let app_h = app.clone();
    let rt_h = Arc::clone(rt);
    let audio_h = audio.clone();
    spawn_action_run(&app_h.clone(), &rt_h.clone(), &audio_h.clone(), move || {
        let runtime = follow_up_runtime(
            &app_h,
            &rt_h,
            &audio_h,
            executing_session_id,
            cancel_flag,
        );
        execute_command_with_context(
            &node,
            &runtime,
            Some(app_index_snapshot.as_slice()),
            tool_context,
            Some(target_aliases_snapshot.as_slice()),
        );
    });
    Ok(())
}

fn tier2_prerequisites(app: &AppHandle) -> Option<(db::AppSettings, String)> {
    if !cfg!(feature = "llm-local") {
        debug!("flow: tier2 unavailable (llm-local not compiled)");
        return None;
    }
    let conn = open_db_connection(app).ok()?;
    let settings = get_app_settings(&conn).ok()?;
    if !settings.llm_router_tier2_enabled {
        debug!("flow: no trigger; tier2 disabled");
        return None;
    }
    let model_path =
        resolve_router_model_path(app, settings.llm_router_model_path.as_deref()).ok()?;
    Some((settings, model_path.display().to_string()))
}

pub(crate) fn route_transcript_tier2(
    app: &AppHandle,
    transcript: &str,
) -> Result<RouterRouteResult, RouterError> {
    let conn = open_db_connection(app).map_err(|message| RouterError {
        code: RouterErrorCode::SchemaInvalid,
        message,
    })?;
    let settings = get_app_settings(&conn).map_err(|e| RouterError {
        code: RouterErrorCode::SchemaInvalid,
        message: e.to_string(),
    })?;
    if !settings.llm_router_tier2_enabled {
        return Err(RouterError {
            code: RouterErrorCode::Tier2Disabled,
            message: "Tier 2 LLM routing is disabled in settings.".into(),
        });
    }
    if !crate::llm::router_model_ready(app) {
        let _ = crate::llm::spawn_router_preload(app);
        return Err(RouterError {
            code: RouterErrorCode::ModelLoading,
            message: crate::llm::ROUTER_LOADING_STATUS.into(),
        });
    }
    let model_path = resolve_router_model_path(app, settings.llm_router_model_path.as_deref())
        .map_err(|message| RouterError {
            code: RouterErrorCode::ModelMissing,
            message,
        })?;
    let tools: Vec<_> = get_all_tools(&conn)
        .map_err(|e| RouterError {
            code: RouterErrorCode::SchemaInvalid,
            message: e.to_string(),
        })?
        .into_iter()
        .filter(|t| t.enabled)
        .collect();
    let use_gpu = llm_gpu_runtime_available(llm_compile_backend());
    let infer = LocalInfer {
        model_path: model_path.display().to_string(),
        use_gpu,
    };
    route_transcript_with_infer(
        &conn,
        &tools,
        transcript.trim(),
        settings.llm_router_confidence_threshold,
        &infer,
    )
}

fn execute_tier2_route(
    app: &AppHandle,
    rt: &SharedHud,
    audio: &SharedAudioPipeline,
    route: RouterRouteResult,
) -> Result<(), String> {
    route_commit_gate(rt)?;
    let app_index_snapshot = {
        let st = app.state::<AppIndexStore>();
        let guard = st
            .read()
            .map_err(|_| "app index lock poisoned".to_string())?;
        guard.clone()
    };
    let aliases = open_db_connection(app)
        .ok()
        .and_then(|conn| get_all_target_aliases(&conn).ok())
        .unwrap_or_default();
    let route = RouterRouteResult {
        tool_calls: normalize_router_tool_calls(
            route.tool_calls,
            app_index_snapshot.as_slice(),
            &aliases,
        )
        .map_err(|e| e.message)?,
        confidence: route.confidence,
    };
    info!(
        "flow: ROUTE steps={} confidence={:.2}",
        route.tool_calls.len(),
        route.confidence
    );
    let _ = app.emit(
        "router-result",
        serde_json::json!({
            "tool_calls": route.tool_calls,
            "confidence": route.confidence,
        }),
    );
    let (executing_session_id, cancel_flag) = begin_execution_session(app, rt, audio, true)?;
    let app_h = app.clone();
    let rt_h = Arc::clone(rt);
    let audio_h = audio.clone();
    let tool_calls = route.tool_calls;
    spawn_action_run(&app_h.clone(), &rt_h.clone(), &audio_h.clone(), move || {
        let runtime = follow_up_runtime(
            &app_h,
            &rt_h,
            &audio_h,
            executing_session_id,
            cancel_flag,
        );
        let conn = match open_db_connection(&app_h) {
            Ok(c) => c,
            Err(err) => {
                runtime.emit_error(&err);
                return;
            }
        };
        for (idx, call) in tool_calls.iter().enumerate() {
            if runtime.is_cancelled() {
                runtime.emit_status(ACTION_RUN_CANCELLED_MSG);
                return;
            }
            info!(
                "flow: ROUTE step {}/{} tool={} args={:?}",
                idx + 1,
                tool_calls.len(),
                call.tool,
                call.args
            );
            if let Err(err) = execute_tool(
                &conn,
                &call.tool,
                &call.args,
                &runtime,
                Some(app_index_snapshot.as_slice()),
            ) {
                if err != ACTION_RUN_CANCELLED_MSG {
                    runtime.emit_error(&err);
                }
                return;
            }
        }
    });
    Ok(())
}

fn begin_tier2_routing(
    app: &AppHandle,
    rt: &SharedHud,
    audio: &SharedAudioPipeline,
) -> Result<u64, String> {
    listening_gate(rt)?;
    debug!("flow: tier2 routing");
    audio::stop_shared_pipeline(app, audio);
    let routing_session_id = set_phase(app, rt, HudPhase::Routing)?;
    let _ = app.emit(
        "action-status",
        serde_json::json!({ "text": "Understanding…" }),
    );
    Ok(routing_session_id)
}

/// Tier 0: deterministic `open {target}` preflight before phrase match / LLM.
pub(crate) fn tier0_open_classification(
    text: &str,
    app_index: &[crate::apps::AppEntry],
    aliases: &[db::TargetAlias],
) -> Option<Result<OpenIntent, String>> {
    let (target, placement) = parse_open_intent(text)?;
    let intent = classify_open_target(&target, placement, app_index, aliases);
    Some(match intent {
        OpenIntent::Unknown { target } => Err(format!("Couldn't find {target}")),
        other => Ok(other),
    })
}

fn try_tier0_open_intent(
    app: &AppHandle,
    rt: &SharedHud,
    audio: &SharedAudioPipeline,
    text: &str,
) -> Option<Result<(), String>> {
    let app_index_snapshot = {
        let st = app.state::<AppIndexStore>();
        let guard = st.read().ok()?;
        guard.clone()
    };
    let aliases = open_db_connection(app)
        .ok()
        .and_then(|conn| get_all_target_aliases(&conn).ok())
        .unwrap_or_default();
    let classification = tier0_open_classification(text, app_index_snapshot.as_slice(), &aliases)?;

    listening_gate(rt).ok()?;
    info!("flow: TIER0 open intent");

    if let Err(message) = &classification {
        let _ = set_phase(app, rt, HudPhase::Matched).ok()?;
        let _ = app.emit("action-error", serde_json::json!({ "message": message }));
        finalize_command_run(app, rt, audio);
        return Some(Ok(()));
    }
    let intent = classification.expect("checked Err branch above");

    let _ = set_phase(app, rt, HudPhase::Matched).ok()?;

    let (executing_session_id, cancel_flag) = begin_execution_session(app, rt, audio, false).ok()?;
    let app_h = app.clone();
    let rt_h = Arc::clone(rt);
    let audio_h = audio.clone();
    let index = app_index_snapshot;
    let alias_snapshot = aliases;

    spawn_action_run(&app_h.clone(), &rt_h.clone(), &audio_h.clone(), move || {
        let runtime = follow_up_runtime(
            &app_h,
            &rt_h,
            &audio_h,
            executing_session_id,
            cancel_flag,
        );
        let conn = match open_db_connection(&app_h) {
            Ok(c) => c,
            Err(err) => {
                runtime.emit_error(&err);
                return;
            }
        };
        let result = match intent {
            OpenIntent::App { target, placement, .. } => execute_open_target_with_aliases(
                &target,
                placement.as_deref(),
                &alias_snapshot,
                &runtime,
                Some(index.as_slice()),
            ),
            OpenIntent::Url { url, placement } => {
                use std::collections::HashMap;
                let mut args = HashMap::from([("url".to_string(), url)]);
                if let Some(p) = placement.filter(|z| !z.trim().is_empty()) {
                    args.insert("placement".to_string(), p);
                }
                execute_tool(&conn, "open_url", &args, &runtime, Some(index.as_slice()))
                    .map(|_| "Opening URL…".to_string())
            }
            OpenIntent::Unknown { .. } => unreachable!("tier0 rejects Unknown before execution"),
        };
        if let Err(err) = result {
            if err != ACTION_RUN_CANCELLED_MSG {
                runtime.emit_error(&err);
            }
        }
    });
    Some(Ok(()))
}

/// Tier 0 open preflight → Tier 1 trigger match → Tier 2 LLM router → tool execution (Tier 3 clarify inside `open_target`).
pub fn try_route_and_execute(
    app: &AppHandle,
    rt: &SharedHud,
    audio: &SharedAudioPipeline,
    text: &str,
) -> Result<(), String> {
    if let Some(tier0) = try_tier0_open_intent(app, rt, audio, text) {
        return tier0;
    }

    let command_cache = app.state::<CommandCache>();
    let nodes = read_command_cache(&command_cache)?;
    let default_threshold_pct = load_default_fuzzy_threshold_pct(app);
    let matcher_nodes = matcher_nodes(&nodes, default_threshold_pct);

    if let Some(matched) = match_command(text, &matcher_nodes) {
        return execute_tier1_match(app, rt, audio, &nodes, matched);
    }

    let Some((_settings, _model_path)) = tier2_prerequisites(app) else {
        debug!("flow: no trigger phrase matched");
        return Ok(());
    };

    if !crate::llm::router_model_ready(app) {
        let _ = crate::llm::spawn_router_preload(app);
        crate::llm::tauri_cmds::emit_router_loading_notice(app);
        debug!("flow: tier2 gated until router model is warm");
        return Ok(());
    }

    let routing_session_id = match begin_tier2_routing(app, rt, audio) {
        Ok(id) => id,
        Err(err) if err == "skip" => return Ok(()),
        Err(err) => return Err(err),
    };

    let app_bg = app.clone();
    let rt_bg = Arc::clone(rt);
    let audio_bg = audio.clone();
    let transcript = text.to_string();
    std::thread::spawn(move || {
        let still_routing = rt_bg
            .lock()
            .map(|s| s.session_id == routing_session_id && s.phase == HudPhase::Routing)
            .unwrap_or(false);
        if !still_routing {
            debug!("flow: skip tier2 (session changed before infer)");
            return;
        }

        match route_transcript_tier2(&app_bg, &transcript) {
            Ok(route) => {
                if let Err(err) = execute_tier2_route(&app_bg, &rt_bg, &audio_bg, route) {
                    if err != "skip" {
                        warn!("flow: tier2 execute failed: {err}");
                    }
                }
            }
            Err(RouterError {
                code: RouterErrorCode::LowConfidence,
                message,
            }) => {
                debug!("flow: tier2 low confidence: {message}");
                finalize_command_run(&app_bg, &rt_bg, &audio_bg);
            }
            Err(RouterError {
                code: RouterErrorCode::ModelLoading,
                message,
            }) => {
                debug!("flow: tier2 gated: {message}");
            }
            Err(err) => {
                debug!("flow: tier2 route miss: {} ({:?})", err.message, err.code);
                finalize_command_run(&app_bg, &rt_bg, &audio_bg);
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::intent::OpenIntent;
    use crate::apps::AppEntry;
    use crate::commands::router::tier0_open_classification;
    use crate::llm::intent_validate::normalize_router_tool_calls;
    use crate::db::{init_db, Action, MatchMode};
    use crate::llm::router::RouterInfer;
    use rusqlite::Connection;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    struct MockInfer(&'static str);

    impl RouterInfer for MockInfer {
        fn infer(&self, _prompt: &str) -> Result<String, String> {
            Ok(self.0.to_string())
        }
    }

    fn test_conn() -> (tempfile::TempDir, Connection) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("router-orchestrator.db");
        init_db(&path).unwrap();
        (dir, Connection::open(&path).unwrap())
    }

    fn open_node(id: i64) -> CommandNode {
        CommandNode {
            id,
            name: "Open".into(),
            trigger_phrases: vec!["open".into()],
            actions: vec![Action::OpenTarget {
                target: "{{remainder}}".into(),
                placement: None,
            }],
            enabled: true,
            fuzzy_threshold_pct: 80,
            match_mode: MatchMode::Prefix,
            created_at: "now".into(),
        }
    }

    #[test]
    fn tier1_prefix_match_extracts_remainder() {
        let nodes = vec![open_node(1)];
        let prepared = matcher_nodes(&nodes, 80);
        let matched = match_command("open notepad", &prepared).expect("tier1 match");
        assert_eq!(matched.remainder, "notepad");
    }

    #[test]
    fn tier1_miss_without_prefix_trigger() {
        let nodes = vec![open_node(1)];
        let prepared = matcher_nodes(&nodes, 80);
        assert!(match_command("put slack on the left", &prepared).is_none());
    }

    #[test]
    fn tier2_route_open_target_left() {
        let (_dir, conn) = test_conn();
        let tool = db::get_tool_by_name(&conn, "open_target")
            .unwrap()
            .expect("builtin");
        let mock = MockInfer(
            r#"{"tool":"open_target","args":{"target":"slack","placement":"left_half"},"confidence":0.91}"#,
        );
        let result = route_transcript_with_infer(
            &conn,
            &[tool],
            "put slack on the left",
            0.7,
            &mock,
        )
        .expect("route");
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].tool, "open_target");
        assert_eq!(
            result.tool_calls[0].args.get("target").map(String::as_str),
            Some("slack")
        );
        assert_eq!(
            result.tool_calls[0]
                .args
                .get("placement")
                .map(String::as_str),
            Some("left_half")
        );
    }

    #[derive(Default)]
    struct MockRuntimeState {
        statuses: Vec<String>,
        errors: Vec<String>,
    }

    #[derive(Clone, Default)]
    struct MockRuntime {
        state: Arc<Mutex<MockRuntimeState>>,
    }

    impl ActionRuntime for MockRuntime {
        fn open_app(&self, _path: &str) -> Result<(), String> {
            Ok(())
        }

        fn open_url(&self, _url: &str) -> Result<(), String> {
            Ok(())
        }

        fn run_script(&self, _script: &str, _args: &[String]) -> Result<(), String> {
            Ok(())
        }

        fn send_keys(&self, _keys: &str) -> Result<(), String> {
            Ok(())
        }

        fn wait_ms(&self, _ms: u64) -> Result<(), String> {
            Ok(())
        }

        fn speak(&self, _text: &str) -> Result<(), String> {
            Ok(())
        }

        fn request_follow_up(&self, _prompt: &str) -> Result<String, String> {
            Err("not configured".into())
        }

        fn is_cancelled(&self) -> bool {
            false
        }

        fn emit_status(&self, text: &str) {
            self.state.lock().unwrap().statuses.push(text.to_string());
        }

        fn emit_error(&self, message: &str) {
            self.state.lock().unwrap().errors.push(message.to_string());
        }
    }

    #[test]
    fn tier0_open_brave_resolves_to_app_intent() {
        let index = vec![AppEntry {
            display_name: "Brave".into(),
            exe_path: r"C:\Brave\brave.exe".into(),
            icon_data_url: None,
        }];
        let result = tier0_open_classification("open brave", &index, &[]).expect("tier0 hit");
        assert!(matches!(result, Ok(OpenIntent::App { .. })));
    }

    #[test]
    fn tier0_open_unknown_fails_fast() {
        let result = tier0_open_classification("open foobar", &[], &[]).expect("tier0 hit");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("foobar"));
    }

    #[test]
    fn tier0_skips_non_open_phrasing() {
        assert!(tier0_open_classification("put slack on the left", &[], &[]).is_none());
    }

    #[test]
    fn tier2_post_validate_rewrites_open_url_app_name() {
        let index = vec![AppEntry {
            display_name: "Brave".into(),
            exe_path: r"C:\Brave\brave.exe".into(),
            icon_data_url: None,
        }];
        let calls = vec![crate::llm::router::RouterToolCall {
            tool: "open_url".into(),
            args: HashMap::from([("url".into(), "brave".into())]),
        }];
        let normalized = normalize_router_tool_calls(calls, &index, &[]).expect("normalized");
        assert_eq!(normalized[0].tool, "open_target");
    }

    #[test]
    fn execute_tool_open_target_from_router_args() {
        let (_dir, conn) = test_conn();
        let runtime = MockRuntime::default();
        let index = vec![AppEntry {
            display_name: "GitHub".into(),
            exe_path: "GitHubDesktop.exe".into(),
            icon_data_url: None,
        }];
        let args = HashMap::from([
            ("target".to_string(), "github".to_string()),
            ("placement".to_string(), "right_half".to_string()),
        ]);
        execute_tool(&conn, "open_target", &args, &runtime, Some(index.as_slice())).expect("tool run");
        assert!(runtime.state.lock().unwrap().errors.is_empty());
    }
}
