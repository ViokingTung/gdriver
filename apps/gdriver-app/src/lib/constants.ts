/**
 * Default Google Drive free-tier storage limit in bytes (15 GB).
 * Used as a display fallback when the actual quota is unavailable.
 */
export const DEFAULT_STORAGE_LIMIT_BYTES = 15 * 1024 * 1024 * 1024;

/**
 * Human-readable default storage limit for display (e.g. "15.00").
 */
export const DEFAULT_STORAGE_LIMIT_GB = (
  DEFAULT_STORAGE_LIMIT_BYTES / (1024 * 1024 * 1024)
).toFixed(2);

/** Default bandwidth rate limit (KB/s) when user enables rate limiting. */
export const DEFAULT_RATE_LIMIT_KBPS = 500;

/** Default hotkey for search — platform-aware (Cmd on macOS, Ctrl elsewhere). */
export const DEFAULT_SEARCH_HOTKEY =
  typeof navigator !== "undefined" && /Mac|iPod|iPhone|iPad/.test(navigator.platform)
    ? "Cmd+Alt+G"
    : "Ctrl+Alt+G";
