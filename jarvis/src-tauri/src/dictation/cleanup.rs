//! S1-mini by Superwhisper transcript normalization.
use std::time::Duration;

pub fn style(value: &str) -> &str {
    match value { "casual" | "semi-casual" | "semi-formal" | "formal" => value, _ => "semi-formal" }
}
pub fn context_for_exe(exe: &str) -> &'static str {
    let name = exe.rsplit(['/', '\\']).next().unwrap_or(exe);
    if ["outlook.exe", "olk.exe", "thunderbird.exe", "hxoutlook.exe"].iter().any(|c| name.eq_ignore_ascii_case(c)) { "email" } else { "general" }
}
fn prompt(raw: &str, styling: &str, context: &str) -> String {
    let context = if context == "email" { "email" } else { "general" };
    format!("<|im_start|>system\nYou are a text normalizer for speech-to-text transcripts. The input begins with a control line specifying the styling, structure, and context settings; clean the transcript to match those settings and output only the cleaned text.<|im_end|>\n<|im_start|>user\n[Styling: {}] [Structure: prose] [Context: {}]\n{}<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n", style(styling), context, raw)
}
fn guard(raw: &str, result: Result<String, String>) -> String {
    let Ok(output) = result else { return raw.to_string(); };
    let output = output.trim();
    if output.is_empty() { return if raw.split_whitespace().count() <= 4 { String::new() } else { raw.to_string() }; }
    let raw_len = raw.chars().count();
    let output_len = output.chars().count();
    if output_len * 10 > raw_len * 16 || output_len * 10 < raw_len * 4 { raw.to_string() } else { output.to_string() }
}
pub fn cleanup(raw: &str, styling: &str, context: &str, use_gpu: bool) -> String {
    let result = (|| {
        let port = crate::llm::local::dictation_server_port(use_gpu).ok_or("dictation server unavailable")?;
        let body = serde_json::json!({ "prompt": prompt(raw, styling, context), "n_predict": raw.split_whitespace().count() * 2 + 32, "temperature": 0, "cache_prompt": true, "stop": ["<|im_end|>"] });
        let response = ureq::AgentBuilder::new().timeout(Duration::from_millis(800)).build().post(&format!("http://127.0.0.1:{port}/completion")).set("Content-Type", "application/json").send_string(&body.to_string()).map_err(|e| e.to_string())?;
        let text = response.into_string().map_err(|e| e.to_string())?;
        let json: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        json.get("content").and_then(|v| v.as_str()).map(str::to_owned).ok_or_else(|| "missing content".to_string())
    })();
    guard(raw, result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_prompt() {
        assert_eq!(prompt("hello world", "semi-formal", "general"), "<|im_start|>system\nYou are a text normalizer for speech-to-text transcripts. The input begins with a control line specifying the styling, structure, and context settings; clean the transcript to match those settings and output only the cleaned text.<|im_end|>\n<|im_start|>user\n[Styling: semi-formal] [Structure: prose] [Context: general]\nhello world<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n");
    }
    #[test]
    fn guard_outputs() {
        assert_eq!(guard("one two three four five", Err("timeout".into())), "one two three four five");
        assert_eq!(guard("um uh", Ok(" ".into())), "");
        assert_eq!(guard("one two three four five", Ok("".into())), "one two three four five");
        assert_eq!(guard("hello world", Ok("a much much longer output".into())), "hello world");
        assert_eq!(guard("hello world", Ok("hi".into())), "hello world");
        assert_eq!(guard("hello world", Ok(" Hello world. ".into())), "Hello world.");
    }
    #[test]
    fn context_and_style_are_restricted() {
        assert_eq!(context_for_exe(r"C:\Program Files\OUTLOOK.EXE"), "email");
        assert_eq!(context_for_exe("chrome.exe"), "general");
        assert_eq!(style("invalid"), "semi-formal");
        for value in ["casual", "semi-casual", "semi-formal", "formal"] { assert_eq!(style(value), value); }
    }
}
