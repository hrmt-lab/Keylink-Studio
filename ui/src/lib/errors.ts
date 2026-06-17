import type { TranslationKey } from "../i18n";

type TFn = (key: TranslationKey, params?: Record<string, string | number>) => string;

/**
 * Convert a raw caught error (or backend error string) into a human-readable,
 * localized message. Known low-level patterns (permission, busy device, broken
 * connection) map to specific guidance; everything else falls back to the
 * caller-provided context message, or a generic one.
 *
 * Mirrors the intent of KeymapViewer's `errorLabel`, but for arbitrary errors
 * surfaced across the app so users never see raw `os error 5`-style strings.
 */
export function friendlyError(
  e: unknown,
  t: TFn,
  fallback: TranslationKey = "error.generic",
): string {
  const raw = String(e).toLowerCase();

  if (
    raw.includes("os error 5") ||
    raw.includes("access is denied") ||
    raw.includes("permission denied")
  ) {
    return t("error.permission_denied");
  }

  if (raw.includes("busy") || raw.includes("in use") || raw.includes("port_busy")) {
    return t("error.device_busy");
  }

  if (
    raw.includes("disconnect") ||
    raw.includes("timed out") ||
    raw.includes("timeout") ||
    raw.includes("connection")
  ) {
    return t("error.connection_failed");
  }

  return t(fallback);
}
