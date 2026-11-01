//! Router (decider) eval: accuracy + latency of candidate GGUF models on fixed voice phrases.
//!
//! Live run (CPU by default; `ROUTER_EVAL_GPU=1` for CUDA):
//! `ROUTER_EVAL_MODELS="C:\models\a.gguf;C:\models\b.gguf" cargo test router_live_eval -- --ignored --nocapture`

use crate::db::{get_all_tools, init_db, ToolDefinition};
use crate::llm::local::{
    choose_option_probs_at, complete_router_at, spawn_router_sidecar_at, ROUTER_MODEL_FILE,
};
use crate::llm::prompt::{build_router_prompt, build_router_schema};
use crate::llm::router::{parse_router_output, validate_router_call, RouterToolCall};
use rusqlite::Connection;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Instant;

const FIXTURES: &str = include_str!("router_eval_cases.json");
const THRESHOLD: f32 = crate::db::settings::DEFAULT_LLM_ROUTER_CONFIDENCE_THRESHOLD;

#[derive(Debug, Deserialize)]
struct Case {
    say: String,
    #[serde(default)]
    focus: Option<String>,
    /// Empty = nothing should run (low confidence or rejected).
    expect: Vec<ExpectCall>,
}

#[derive(Debug, Deserialize)]
struct ExpectCall {
    tool: String,
    /// Each expected value must appear (case-insensitive substring) in the model's arg.
    #[serde(default)]
    args: HashMap<String, String>,
}

fn cases() -> Vec<Case> {
    // ROUTER_EVAL_CASES=<path> scores a held-out file instead of the tuned fixtures.
    let text = std::env::var("ROUTER_EVAL_CASES")
        .map(|path| std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}")))
        .unwrap_or_else(|_| FIXTURES.to_string());
    serde_json::from_str(&text).expect("eval cases parse")
}

fn seeded_db() -> (tempfile::TempDir, Connection, Vec<ToolDefinition>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("router-eval.db");
    init_db(&path).unwrap();
    let conn = Connection::open(&path).unwrap();
    crate::db::upsert_registered_script(
        &conn,
        &crate::db::RegisteredScript {
            id: "morning_routine".into(),
            path: "eval-fixture-only".into(),
            hash: "eval-fixture-only".into(),
            display_name: "Morning routine".into(),
            arg_names: Vec::new(),
        },
    )
    .unwrap();
    let tools = get_all_tools(&conn).unwrap();
    (dir, conn, tools)
}

fn tools_match(expect: &[ExpectCall], got: &[RouterToolCall]) -> bool {
    expect.len() == got.len() && expect.iter().zip(got).all(|(e, g)| e.tool == g.tool)
}

fn args_match(expect: &[ExpectCall], got: &[RouterToolCall]) -> bool {
    tools_match(expect, got)
        && expect.iter().zip(got).all(|(e, g)| {
            e.args.iter().all(|(k, v)| {
                g.args
                    .get(k)
                    .is_some_and(|actual| actual.to_lowercase().contains(&v.to_lowercase()))
            })
        })
}

fn percentile(sorted_ms: &[u128], p: f64) -> u128 {
    let idx = ((sorted_ms.len() as f64 - 1.0) * p).round() as usize;
    sorted_ms[idx]
}

/// Offline: fixtures reference real tools and valid enum values (catches fixture drift).
#[test]
fn fixtures_match_seeded_tool_catalog() {
    let (_dir, _conn, tools) = seeded_db();
    for case in cases() {
        for call in &case.expect {
            let tool = tools
                .iter()
                .find(|t| t.name == call.tool)
                .unwrap_or_else(|| panic!("`{}`: unknown tool `{}`", case.say, call.tool));
            for (key, value) in &call.args {
                let param = tool
                    .parameters
                    .iter()
                    .find(|p| &p.name == key)
                    .unwrap_or_else(|| {
                        panic!("`{}`: `{}` has no param `{key}`", case.say, tool.name)
                    });
                if param.param_type == "enum" {
                    assert!(
                        param.enum_values.contains(value),
                        "`{}`: `{value}` not in {key} enum",
                        case.say
                    );
                }
            }
        }
    }
}

#[test]
fn scoring_requires_order_and_arg_substring() {
    let expect = vec![ExpectCall {
        tool: "open_url".into(),
        args: HashMap::from([("url".into(), "github.com".into())]),
    }];
    let got = vec![RouterToolCall {
        tool: "open_url".into(),
        args: HashMap::from([("url".into(), "https://GitHub.com".into())]),
    }];
    assert!(args_match(&expect, &got));
    assert!(!args_match(&expect, &[]));
    let wrong = vec![RouterToolCall {
        tool: "open_url".into(),
        args: HashMap::from([("url".into(), "https://gitlab.com".into())]),
    }];
    assert!(tools_match(&expect, &wrong) && !args_match(&expect, &wrong));
}

fn eval_env() -> (std::path::PathBuf, String, bool) {
    let resource_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources");
    let models = std::env::var("ROUTER_EVAL_MODELS")
        .unwrap_or_else(|_| resource_root.join(ROUTER_MODEL_FILE).display().to_string());
    let use_gpu = std::env::var("ROUTER_EVAL_GPU").is_ok_and(|v| v == "1");
    (resource_root, models, use_gpu)
}

/// Example phrasings per tool for the decision prompt (what the tool's description would carry).
fn decision_prompt(transcript: &str, options: &[String], tools: &[ToolDefinition]) -> String {
    let options = options
        .iter()
        .map(|name| {
            if name == "NONE" {
                format!("{name}: chit-chat or nonsense")
            } else {
                format!(
                    "{name}: {}",
                    tools
                        .iter()
                        .find(|tool| tool.name == *name)
                        .map_or("", |tool| tool.description.as_str())
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("Pick the best computer action or NONE.\n{options}\nVoice command: \"{transcript}\"")
}

/// Decision-style routing: one forced letter, real per-option probabilities, threshold on the top one.
/// Scores the first tool only (multi-step cases count their first step).
#[test]
#[ignore = "requires GGUF models and the llama-server sidecar"]
fn router_decision_eval() {
    let (resource_root, models, use_gpu) = eval_env();
    let (_dir, _conn, tools) = seeded_db();
    let mut options: Vec<String> = tools
        .iter()
        .filter(|t| t.enabled)
        .map(|t| t.name.clone())
        .collect();
    options.sort();
    options.push("NONE".into());
    let cases: Vec<_> = cases().into_iter().filter(|case| case.focus.is_none()).collect();
    let thresholds = [0.0, 0.5, 0.7];

    println!(
        "\ndecision eval: {} cases, {} options, gpu={use_gpu}",
        cases.len(),
        options.len()
    );
    println!(
        "{:<44} {:>10} {:>10} {:>10} {:>7} {:>7}",
        "model", "acc@0.0", "acc@0.5", "acc@0.7", "p50ms", "p95ms"
    );
    for model in models.split(';').map(str::trim).filter(|m| !m.is_empty()) {
        let name = std::path::Path::new(model)
            .file_name()
            .map_or(model.to_string(), |n| n.to_string_lossy().into_owned());
        let sidecar = match spawn_router_sidecar_at(&resource_root, model, use_gpu) {
            Ok(s) => s,
            Err(err) => {
                println!("{name:<44} FAILED TO START: {err}");
                continue;
            }
        };
        let system = "You classify voice commands. Reply with one letter.";
        let n = options.len();
        // AnyJev-style debiasing: average over cyclic option rotations, divide out the
        // content-free label prior (same prompt with an empty command).
        let rotations: usize = std::env::var("ROUTER_EVAL_ROTATIONS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1)
            .clamp(1, n);
        let calibrate = std::env::var("ROUTER_EVAL_CALIBRATE").is_ok_and(|v| v == "1");
        let score = |say: &str| -> Result<Vec<f64>, String> {
            let mut avg = vec![0.0; n];
            for r in 0..rotations {
                let shift = r * n / rotations;
                let rotated: Vec<String> =
                    (0..n).map(|i| options[(i + shift) % n].clone()).collect();
                let mut probs = choose_option_probs_at(
                    sidecar.port,
                    system,
                    &decision_prompt(say, &rotated, &tools),
                    n,
                )?;
                if calibrate {
                    let prior = choose_option_probs_at(
                        sidecar.port,
                        system,
                        &decision_prompt("", &rotated, &tools),
                        n,
                    )?;
                    probs
                        .iter_mut()
                        .zip(&prior)
                        .for_each(|(p, q)| *p /= q.max(1e-4));
                    let total: f64 = probs.iter().sum();
                    probs.iter_mut().for_each(|p| *p /= total);
                }
                for (i, p) in probs.iter().enumerate() {
                    avg[(i + shift) % n] += p / rotations as f64;
                }
            }
            Ok(avg)
        };
        let _ = score("open notepad");
        println!("  (rotations={rotations}, calibrate={calibrate})");
        let mut correct = [0usize; 3];
        let mut latencies = Vec::new();
        let mut failures = Vec::new();
        for case in &cases {
            let want = case.expect.first().map_or("NONE", |c| c.tool.as_str());
            let start = Instant::now();
            let probs = score(&case.say);
            latencies.push(start.elapsed().as_millis());
            let Ok(probs) = probs else {
                failures.push(format!("  `{}` → {}", case.say, probs.unwrap_err()));
                continue;
            };
            let (best, p) =
                probs.iter().enumerate().fold(
                    (0, 0.0),
                    |acc, (i, &p)| if p > acc.1 { (i, p) } else { acc },
                );
            for (slot, t) in thresholds.iter().enumerate() {
                let pick = if p >= *t {
                    options[best].as_str()
                } else {
                    "NONE"
                };
                correct[slot] += usize::from(pick == want);
            }
            if options[best] != want || p < 0.5 {
                failures.push(format!(
                    "  `{}` → {} ({p:.2}), want {want}",
                    case.say, options[best]
                ));
            }
        }
        latencies.sort_unstable();
        let pct = |n: usize| 100.0 * n as f64 / cases.len() as f64;
        println!(
            "{name:<44} {:>10.0} {:>10.0} {:>10.0} {:>7} {:>7}",
            pct(correct[0]),
            pct(correct[1]),
            pct(correct[2]),
            percentile(&latencies, 0.5),
            percentile(&latencies, 0.95),
        );
        for line in &failures {
            println!("{line}");
        }
    }
}

/// Decision models (Julia, d1, d3 …) through llama-server `/v1/systemone`.
/// Scores the first expected tool (thresholded on its probability) and, separately,
/// enum args for cases that expect one.
#[test]
#[ignore = "requires decision-model GGUFs and the llama-server sidecar"]
fn router_systemone_eval() {
    let (resource_root, models, use_gpu) = eval_env();
    let (dir, conn, tools) = seeded_db();
    let eval_path = dir.path().join("router-eval.db");
    let cases = cases();
    let threshold = std::env::var("ROUTER_EVAL_THRESHOLD")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(THRESHOLD);
    println!(
        "\nfull decider eval: {} cases, gpu={use_gpu}, threshold={threshold}, variant=two_call",
        cases.len()
    );
    println!(
        "{:<34} {:>7} {:>7} {:>7} {:>11} {:>13} {:>7} {:>7}",
        "model", "cmd%", "tool%", "full%", "NONE rej/total", "infraErrors", "p50ms", "p95ms"
    );
    for model in models.split(';').map(str::trim).filter(|m| !m.is_empty()) {
        let name = std::path::Path::new(model)
            .file_name()
            .map_or(model.to_string(), |n| n.to_string_lossy().into_owned());
        let sidecar = spawn_router_sidecar_at(&resource_root, model, use_gpu)
            .unwrap_or_else(|err| panic!("{name}: failed to start eval sidecar: {err}"));
        crate::llm::decider::decide_eval(&conn, &tools, "open notepad", None, 0.0, sidecar.port)
            .unwrap_or_else(|err| panic!("{name}: warmup failed: {err:?}"));
        for focused in [false, true] {
            let selected: Vec<_> = cases.iter().filter(|case| case.focus.is_some() == focused).collect();
            if selected.is_empty() { continue; }
            let app_only = std::env::var("ROUTER_EVAL_FOCUS").is_ok_and(|v| v == "app");
            println!("set={}, cases={}, focus={}", if focused { "focused" } else { "unfocused" }, selected.len(), if app_only { "app" } else { "title" });
            let mut command_total = 0usize;
            let mut command_ok = 0usize;
            let mut tool_ok = 0usize;
            let mut full_ok = 0usize;
            let mut none_total = 0usize;
            let mut none_ok = 0usize;
            let mut infra_errors = 0usize;
            let mut latencies = Vec::new();
            let mut failures = Vec::new();
            for case in &selected {
                // App names identify failures without exposing private window titles.
                let label = case.focus.as_deref().map_or_else(
                    || case.say.clone(),
                    |focus| format!("{} [{}]", case.say, focus.split(" (window:").next().unwrap_or(focus)),
                );
                // ROUTER_EVAL_FOCUS=none is the control: same phrases, no context.
                let no_context = std::env::var("ROUTER_EVAL_FOCUS").is_ok_and(|v| v == "none");
                let context = case.focus.as_ref().filter(|_| !no_context).map(|focus| {
                    if app_only { focus.split(" (window:").next().unwrap_or(focus).to_string() }
                    else { focus.clone() }
                });
                let started = Instant::now();
                let result =
                    crate::llm::decider::decide_eval(&conn, &tools, &case.say, context.as_deref(), threshold, sidecar.port);
                latencies.push(started.elapsed().as_millis());
                match result {
                    Ok(route) if case.expect.is_empty() => {
                        none_total += 1;
                        failures.push(format!("{} -> unexpected {:?}", label, route.tool_calls));
                    }
                    Ok(route) => {
                        command_total += 1;
                        if tools_match(&case.expect, &route.tool_calls) {
                            command_ok += 1;
                            tool_ok += 1;
                        } else {
                            failures.push(format!("{} -> wrong tool {:?}", label, route.tool_calls));
                        }
                        if args_match(&case.expect, &route.tool_calls) {
                            full_ok += 1;
                        } else {
                            failures.push(format!("{} -> wrong args {:?}", label, route.tool_calls));
                        }
                    }
                    Err(err) if err.code == crate::llm::router::RouterErrorCode::LowConfidence => {
                        if case.expect.is_empty() {
                            none_total += 1;
                            none_ok += 1;
                            tool_ok += 1;
                            full_ok += 1;
                        } else {
                            command_total += 1;
                            failures.push(format!("{} -> LOW_CONFIDENCE ({})", label, err.message));
                        }
                    }
                    Err(err) => {
                        infra_errors += 1;
                        if case.expect.is_empty() {
                            none_total += 1;
                        } else {
                            command_total += 1;
                        }
                        failures.push(format!("{} -> {:?}: {}", label, err.code, err.message));
                    }
                }
            }
            latencies.sort_unstable();
            let pct = |n: usize, d: usize| {
                if d == 0 {
                    0.0
                } else {
                    100.0 * n as f64 / d as f64
                }
            };
            println!(
                "{name:<34} {:>6.1}% {:>6.1}% {:>6.1}% {:>5}/{:<5} {:>13} {:>7} {:>7}",
                pct(command_ok, command_total),
                pct(tool_ok, selected.len()),
                pct(full_ok, selected.len()),
                none_ok,
                none_total,
                infra_errors,
                percentile(&latencies, 0.5),
                percentile(&latencies, 0.95)
            );
            for failure in failures {
                println!("{failure}");
            }
            assert_eq!(
                infra_errors, 0,
                "{name}: evaluator had infrastructure/schema errors"
            );
        }
        if std::env::var("ROUTER_EVAL_CLAUSE_BENCH").is_ok_and(|v| v == "1") {
            benchmark_clause_concurrency(&eval_path, &tools, &cases, threshold, sidecar.port);
        }
    }
}

fn benchmark_clause_concurrency(
    path: &std::path::Path,
    tools: &[ToolDefinition],
    cases: &[Case],
    threshold: f32,
    port: u16,
) {
    use rusqlite::OpenFlags;
    let mut sequential = Vec::new();
    let mut concurrent = Vec::new();
    for case in cases {
        let clauses = crate::llm::decider::split_clauses_for_eval(&case.say);
        if clauses.len() < 2 {
            continue;
        }
        let started = Instant::now();
        for clause in &clauses {
            let result = crate::llm::decider::decide_eval(
                &Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap(),
                tools,
                clause,
                case.focus.as_deref(),
                threshold,
                port,
            );
            assert!(
                result.as_ref().map_or_else(|err| err.code == crate::llm::router::RouterErrorCode::LowConfidence, |_| true),
                "sequential full-decider clause benchmark failed: {result:?}"
            );
        }
        sequential.push(started.elapsed().as_millis());
        let started = Instant::now();
        let failed = std::thread::scope(|scope| {
            let jobs: Vec<_> = clauses
                .iter()
                .map(|clause| {
                    let path = path.to_path_buf();
                    let tools = tools.to_vec();
                    let clause = clause.clone();
                    let context = case.focus.clone();
                    scope.spawn(move || {
                        let conn =
                            Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                                .map_err(|err| err.to_string())?;
                        match crate::llm::decider::decide_eval(&conn, &tools, &clause, context.as_deref(), threshold, port) {
                            Ok(_) => Ok(()),
                            Err(err) if err.code == crate::llm::router::RouterErrorCode::LowConfidence => Ok(()),
                            Err(err) => Err(err.message),
                        }
                    })
                })
                .collect();
            jobs.into_iter()
                .map(|job| job.join())
                .filter(|result| result.as_ref().map_or(true, |result| result.is_err()))
                .count()
        });
        concurrent.push(started.elapsed().as_millis());
        assert_eq!(failed, 0, "concurrent full-decider clause benchmark failed");
    }
    sequential.sort_unstable();
    concurrent.sort_unstable();
    println!("full-decider clause latency n={}: sequential p50={}ms p95={}ms; concurrent p50={}ms p95={}ms",
        sequential.len(), percentile(&sequential, 0.5), percentile(&sequential, 0.95), percentile(&concurrent, 0.5), percentile(&concurrent, 0.95));
}
#[test]
#[ignore = "requires GGUF models and the llama-server sidecar"]
fn router_live_eval() {
    let (resource_root, models, use_gpu) = eval_env();
    let (_dir, conn, tools) = seeded_db();
    let schema = build_router_schema(&tools);
    let cases: Vec<_> = cases().into_iter().filter(|case| case.focus.is_none()).collect();

    println!(
        "\nrouter eval: {} cases, gpu={use_gpu}, threshold={THRESHOLD}",
        cases.len()
    );
    println!(
        "{:<44} {:>6} {:>6} {:>7} {:>7} {:>7}",
        "model", "tool%", "full%", "none%", "p50ms", "p95ms"
    );

    for model in models.split(';').map(str::trim).filter(|m| !m.is_empty()) {
        let name = std::path::Path::new(model)
            .file_name()
            .map_or(model.to_string(), |n| n.to_string_lossy().into_owned());
        let sidecar = match spawn_router_sidecar_at(&resource_root, model, use_gpu) {
            Ok(s) => s,
            Err(err) => {
                println!("{name:<44} FAILED TO START: {err}");
                continue;
            }
        };
        // Warm the prompt cache so latency reflects steady-state voice use.
        let _ = complete_router_at(
            sidecar.port,
            &build_router_prompt("open notepad", &tools),
            &schema,
        );

        let (mut tool_ok, mut full_ok, mut none_ok, mut none_total) = (0, 0, 0, 0);
        let mut latencies = Vec::new();
        let mut failures = Vec::new();
        for case in &cases {
            let prompt = build_router_prompt(&case.say, &tools);
            let start = Instant::now();
            let completion = complete_router_at(sidecar.port, &prompt, &schema);
            latencies.push(start.elapsed().as_millis());
            let routed = completion
                .as_deref()
                .map_err(|e| e.clone())
                .and_then(|raw| parse_router_output(raw).map_err(|e| e.message))
                .and_then(|raw| {
                    validate_router_call(&conn, &raw, THRESHOLD).map_err(|e| e.message)
                });
            let got = routed
                .as_ref()
                .map(|r| r.tool_calls.as_slice())
                .unwrap_or(&[]);
            if case.expect.is_empty() {
                none_total += 1;
                if got.is_empty() {
                    none_ok += 1;
                    tool_ok += 1;
                    full_ok += 1;
                } else {
                    failures.push(format!("  `{}` → expected nothing, got {got:?}", case.say));
                }
                continue;
            }
            tool_ok += usize::from(tools_match(&case.expect, got));
            if args_match(&case.expect, got) {
                full_ok += 1;
            } else {
                let detail = routed
                    .as_ref()
                    .map_or_else(|e| e.clone(), |r| format!("{:?}", r.tool_calls));
                failures.push(format!("  `{}` → {detail}", case.say));
            }
        }
        latencies.sort_unstable();
        let pct = |n: usize, d: usize| {
            if d == 0 {
                0.0
            } else {
                100.0 * n as f64 / d as f64
            }
        };
        println!(
            "{name:<44} {:>6.0} {:>6.0} {:>7.0} {:>7} {:>7}",
            pct(tool_ok, cases.len()),
            pct(full_ok, cases.len()),
            pct(none_ok, none_total),
            percentile(&latencies, 0.5),
            percentile(&latencies, 0.95),
        );
        for line in &failures {
            println!("{line}");
        }
    }
}
