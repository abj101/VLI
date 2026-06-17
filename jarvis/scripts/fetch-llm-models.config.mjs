/**
 * URLs and paths for on-device LLM GGUF weights.
 * Keep in sync with `llm/local.rs` ROUTER_MODEL_FILE / COMPOSER_MODEL_FILE and the download-*.ps1 scripts.
 */
export const llmModels = [
  {
    file: "qwen2.5-0.5b-instruct-q4_k_m.gguf",
    url: "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf",
    label: "router",
  },
  {
    file: "qwen2.5-3b-instruct-q4_k_m.gguf",
    url: "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf",
    label: "composer",
  },
];
