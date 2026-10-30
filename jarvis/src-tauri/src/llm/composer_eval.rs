//! Fixture-driven composer eval harness (mock infer + optional live model).

use crate::llm::composer::{generate_automation_with_infer, ComposerInfer};
use crate::llm::composer_expect::{assert_result_matches, ComposerExpectation};
use crate::llm::local::{
    infer_composer_completion, ComposerInferWorker, COMPOSER_DEFAULT_MAX_TOKENS,
};
use serde::Deserialize;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
struct EvalCase {
    id: String,
    description: String,
    mock_output: String,
    expect: ComposerExpectation,
    #[serde(default)]
    platform: Option<String>,
}

const FIXTURES: &str = include_str!("composer_eval_cases.json");
const BUILTINS: &[&str] = &["open_url", "open_target", "snap_window"];

fn current_platform() -> &'static str {
    std::env::consts::OS
}

fn case_matches_platform(case: &EvalCase) -> bool {
    match case.platform.as_deref() {
        None | Some("") => true,
        Some(required) => required == current_platform(),
    }
}

struct MockInfer(String);

impl ComposerInfer for MockInfer {
    fn infer(&self, _prompt: &str) -> Result<String, String> {
        Ok(self.0.clone())
    }
}

fn load_fixtures() -> Vec<EvalCase> {
    serde_json::from_str(FIXTURES).expect("composer_eval_cases.json must parse")
}

#[test]
fn eval_all_fixtures_with_mock() {
    for case in load_fixtures() {
        if !case_matches_platform(&case) {
            continue;
        }
        let result = generate_automation_with_infer(
            &case.description,
            None,
            &MockInfer(case.mock_output),
            BUILTINS,
        )
        .unwrap_or_else(|err| panic!("case `{}` failed to generate: {}", case.id, err.message));

        assert_result_matches(&result, &case.expect)
            .unwrap_or_else(|err| panic!("case `{}` assertion failed: {err}", case.id));
    }
}

#[test]
fn bad_json_repair_fixture() {
    let bad = r#"{"kind":"command","confidence":0.8,"summary":"clip","command":{"trigger_phrases":["clip"],"match_mode":"phrase","actions":[{"get_clipboard":null}]}}"#;
    let good = r#"{"kind":"command","confidence":0.8,"summary":"clip","command":{"trigger_phrases":["clip"],"match_mode":"phrase","actions":[{"get_clipboard":{}}]}}"#;
    let calls = std::cell::Cell::new(0u32);

    struct RepairMock<'a> {
        bad: &'a str,
        good: &'a str,
        calls: &'a std::cell::Cell<u32>,
    }

    impl ComposerInfer for RepairMock<'_> {
        fn infer(&self, _prompt: &str) -> Result<String, String> {
            let n = self.calls.get();
            self.calls.set(n + 1);
            if n == 0 {
                Ok(self.bad.to_string())
            } else {
                Ok(self.good.to_string())
            }
        }
    }

    let mock = RepairMock {
        bad,
        good,
        calls: &calls,
    };

    let result =
        generate_automation_with_infer("clip", None, &mock, BUILTINS).expect("repair succeeds");
    assert_eq!(calls.get(), 2);
    assert_eq!(result.kind, crate::llm::composer::ComposerKind::Command);
    assert!(result.command.is_some());
}

#[test]
#[ignore = "requires the composer model and llama-server sidecar"]
fn composer_live_eval_all_fixtures() {
    struct LiveInfer<'a> {
        worker: &'a ComposerInferWorker,
    }

    impl ComposerInfer for LiveInfer<'_> {
        fn infer(&self, prompt: &str) -> Result<String, String> {
            infer_composer_completion(
                self.worker,
                prompt,
                COMPOSER_DEFAULT_MAX_TOKENS,
                Arc::new(AtomicBool::new(false)),
            )
        }
    }

    let resource_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources");
    let model_path = resource_root.join("qwen2.5-3b-instruct-q4_k_m.gguf");
    assert!(
        model_path.is_file(),
        "composer model missing: {}",
        model_path.display()
    );
    let use_gpu = cfg!(windows)
        && resource_root.join("bin/llama/cudart64_12.dll").is_file()
        && std::process::Command::new("nvidia-smi")
            .arg("-L")
            .output()
            .is_ok_and(|result| result.status.success());
    let worker = ComposerInferWorker::spawn_with_resource_root(
        &resource_root,
        model_path.display().to_string(),
        use_gpu,
    )
    .unwrap_or_else(|error| panic!("failed to start composer sidecar: {error}"));
    let infer = LiveInfer { worker: &worker };
    let mut passed = 0;
    let mut scored = 0;
    let mut failures = Vec::new();

    for case in load_fixtures() {
        if !case_matches_platform(&case) {
            continue;
        }
        scored += 1;
        match generate_automation_with_infer(&case.description, None, &infer, BUILTINS) {
            Ok(result) => match assert_result_matches(&result, &case.expect) {
                Ok(()) => passed += 1,
                Err(error) => failures.push(format!("{}: {error}", case.id)),
            },
            Err(error)
                if error.message.contains("llama-server")
                    || error.message.contains("timed out") =>
            {
                panic!(
                    "composer inference transport failed for {}: {}",
                    case.id, error.message
                );
            }
            Err(error) => failures.push(format!("{}: {}", case.id, error.message)),
        }
    }

    println!("COMPOSER_EVAL_SCORE {passed}/{scored}");
    for failure in failures {
        println!("COMPOSER_EVAL_MISS {failure}");
    }
}
