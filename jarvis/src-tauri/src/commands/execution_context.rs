use crate::commands::{executor::ToolCallContext, values::Value};
use std::collections::HashMap;

/// Per-run pipeline state for template refs and step-to-step data flow.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExecutionContext {
    pub shortcut_input: Option<Value>,
    pub last_result: Option<Value>,
    pub step_outputs: Vec<Value>,
    pub follow_up_responses: Vec<String>,
    pub locals: HashMap<String, Value>,
}

impl ExecutionContext {
    pub fn from_tool_context(tool_context: Option<&ToolCallContext>) -> Self {
        let shortcut_input = tool_context
            .and_then(|ctx| ctx.args.get("remainder"))
            .map(|s| Value::Text(s.clone()));
        Self {
            shortcut_input,
            ..Self::default()
        }
    }

    pub fn record_step_output(&mut self, value: Value) {
        self.last_result = Some(value.clone());
        self.step_outputs.push(value);
    }

    pub fn step_output_at(&self, one_based_index: usize) -> Option<&Value> {
        if one_based_index == 0 {
            return None;
        }
        self.step_outputs.get(one_based_index - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_input_populated_from_remainder() {
        let ctx = ToolCallContext::with_remainder("hello");
        let exec = ExecutionContext::from_tool_context(Some(&ctx));
        assert_eq!(
            exec.shortcut_input,
            Some(Value::Text("hello".to_string()))
        );
    }

    #[test]
    fn record_step_output_updates_last_result_and_history() {
        let mut exec = ExecutionContext::default();
        exec.record_step_output(Value::Text("a".into()));
        exec.record_step_output(Value::Text("b".into()));
        assert_eq!(exec.last_result, Some(Value::Text("b".into())));
        assert_eq!(exec.step_outputs.len(), 2);
        assert_eq!(exec.step_output_at(1), Some(&Value::Text("a".into())));
        assert_eq!(exec.step_output_at(2), Some(&Value::Text("b".into())));
    }
}
