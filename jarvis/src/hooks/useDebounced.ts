import { useEffect, useState } from "react";

/**
 * Trailing-edge debounce for strings. When `ms <= 0`, returns `value` synchronously.
 */
export function useDebounced(value: string, ms: number): string {
  const [debounced, setDebounced] = useState(value);

  useEffect(() => {
    if (ms <= 0) {
      return;
    }
    const id = window.setTimeout(() => setDebounced(value), ms);
    return () => window.clearTimeout(id);
  }, [value, ms]);

  return ms <= 0 ? value : debounced;
}
