/**
 * Maps backend / Tauri errors to short, user-facing copy. Falls back to a safe generic line.
 */
export function formatUserError(err: unknown, fallback: string): string {
  const raw = err instanceof Error ? err.message : String(err);
  const low = raw.toLowerCase();

  if (!raw.trim()) {
    return fallback;
  }

  if (low.includes("network") || low.includes("econnrefused") || low.includes("connection refused")) {
    return "Could not reach the service. Check your connection and try again.";
  }
  if (low.includes("timeout") || low.includes("timed out")) {
    return "The operation timed out. Try again.";
  }
  if (low.includes("permission") || low.includes("access denied") || low.includes("denied")) {
    return "Permission was denied. Check system settings and try again.";
  }
  if (low.includes("not found") || low.includes("404")) {
    return "The requested resource was not found.";
  }
  if (low.includes("already exists") || low.includes("duplicate")) {
    return "That value already exists. Choose a different one.";
  }
  if (low.includes("invalid") && raw.length < 120) {
    return raw;
  }

  if (raw.length <= 100 && !raw.includes("invoke") && !raw.includes("tauri")) {
    return raw;
  }

  return fallback;
}
