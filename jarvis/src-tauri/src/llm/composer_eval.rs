//! Fixture-driven composer eval harness (mock infer + optional live model).

use crate::llm::composer::{generate_automation_with_infer, ComposerInfer};
use crate::llm::composer_expect::{assert_result_matches, ComposerExpectation};
use serde::Deserialize;

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
        .unwrap_or_else(|err| {
            panic!(
                "case `{}` failed to generate: {}",
                case.id, err.message
            )
        });

        assert_result_matches(&result, &case.expect).unwrap_or_else(|err| {
            panic!("case `{}` assertion failed: {err}", case.id)
        });
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

    let result = generate_automation_with_infer("clip", None, &mock, BUILTINS).expect("repair succeeds");
    assert_eq!(calls.get(), 2);
    assert_eq!(result.kind, crate::llm::composer::ComposerKind::Command);
    assert!(result.command.is_some());
}

#[test]
#[ignore = "requires llm-local feature and composer GGUF model"]
#[cfg(feature = "llm-local")]
fn composer_live_eval_all_fixtures() {
    panic!("composer live eval harness not wired yet — use `npm run test:composer` mock fixtures for CI");
}
