/**
 * Tauri maps the Rust parameter named `payload` to a top-level `payload` key
 * (same pattern as `reorder_commands`). Flat `{ query, limit }` fails to deserialize.
 */
export function searchAppIndexInvokeArgs(query: string, limit: number) {
  return { payload: { query, limit } } as const;
}
