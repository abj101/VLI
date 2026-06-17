use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Typed action output for Shortcuts-style data flow between pipeline steps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Value {
    Text(String),
    Number(f64),
    Bool(bool),
    List(Vec<Value>),
    Dict(BTreeMap<String, Value>),
    FilePath(PathBuf),
    Nothing,
}

impl Value {
    /// Apple-style coercion: any scalar becomes text when an action expects a string.
    pub fn coerce_to_text(&self) -> String {
        match self {
            Value::Text(s) => s.clone(),
            Value::Number(n) => {
                if n.fract() == 0.0 && n.is_finite() {
                    format!("{:.0}", n)
                } else {
                    n.to_string()
                }
            }
            Value::Bool(b) => b.to_string(),
            Value::List(items) => items
                .iter()
                .map(Value::coerce_to_text)
                .collect::<Vec<_>>()
                .join(", "),
            Value::Dict(map) => map
                .iter()
                .map(|(k, v)| format!("{k}={}", v.coerce_to_text()))
                .collect::<Vec<_>>()
                .join(", "),
            Value::FilePath(p) => p.to_string_lossy().into_owned(),
            Value::Nothing => String::new(),
        }
    }

    /// Human-readable preview for HUD status strips and editor test results.
    pub fn display_string(&self) -> String {
        match self {
            Value::Nothing => String::new(),
            other => other.coerce_to_text(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_coerces_to_text_without_decimal_for_whole_values() {
        assert_eq!(Value::Number(42.0).coerce_to_text(), "42");
    }

    #[test]
    fn bool_coerces_to_text() {
        assert_eq!(Value::Bool(true).coerce_to_text(), "true");
    }

    #[test]
    fn file_path_coerces_to_text() {
        let v = Value::FilePath(PathBuf::from(r"C:\temp\file.txt"));
        assert_eq!(v.coerce_to_text(), r"C:\temp\file.txt");
    }

    #[test]
    fn dict_coerces_to_json_with_numbers() {
        let mut map = BTreeMap::new();
        map.insert("total_memory_bytes".into(), Value::Number(16_000_000_000.0));
        let text = Value::Dict(map).coerce_to_text();
        assert!(text.contains("total_memory_bytes="), "got {text}");
    }
}
