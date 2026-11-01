/**
 * URLs and paths for on-device LLM GGUF weights.
 * Keep in sync with `llm/local.rs` ROUTER_MODEL_FILE / COMPOSER_MODEL_FILE and the download-*.ps1 scripts.
 */
export const llmModels = [
  {
    file: "d1-2B-Q4_K_M.gguf",
    url: "https://huggingface.co/LiquidAI/d1-2B-GGUF/resolve/main/d1-2B-Q4_K_M.gguf",
    label: "router",
  },
  {
    file: "qwen2.5-3b-instruct-q4_k_m.gguf",
    url: "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf",
    label: "composer",
  },
];
