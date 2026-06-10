import type { Lang, TranslationKey } from "../i18n";
import type { AiUsageStatusKind } from "../types";

const LOCALE: Record<Lang, string> = { ja: "ja-JP", en: "en-US" };

/** Type-safe map from an AI usage status to its translation key. */
const AI_STATUS_KEYS: Record<AiUsageStatusKind, TranslationKey> = {
  disabled: "ai_usage.status.disabled",
  ok: "ai_usage.status.ok",
  stale: "ai_usage.status.stale",
  no_data: "ai_usage.status.no_data",
  missing_credentials: "ai_usage.status.missing_credentials",
  expired_credentials: "ai_usage.status.expired_credentials",
  auth_failed: "ai_usage.status.auth_failed",
  rate_limited: "ai_usage.status.rate_limited",
  fetch_failed: "ai_usage.status.fetch_failed",
  parse_failed: "ai_usage.status.parse_failed",
  missing_limit: "ai_usage.status.missing_limit",
};

export function aiStatusKey(status: AiUsageStatusKind): TranslationKey {
  return AI_STATUS_KEYS[status];
}

/** Format a "used" basis-points value (0–10000) as a percentage string. */
export function formatUsedBp(bp: number): string {
  return `${(bp / 100).toFixed(2)}%`;
}

/** Tailwind background class for a usage bar, based on used basis points. */
export function usageBarColor(
  bp: number,
  valid: boolean,
  accent: "primary" | "amber" = "primary"
): string {
  if (!valid) return "bg-gray-300";
  if (bp >= 9000) return "bg-red-500";
  if (bp >= 8000) return "bg-orange-400";
  return accent === "amber" ? "bg-amber-600" : "bg-primary";
}

/** Inline color (hex) for usage text, based on used basis points. */
export function usageTextColor(bp: number, accent: "primary" | "amber" = "primary"): string {
  if (bp >= 9000) return "#ef4444";
  if (bp >= 8000) return "#d97706";
  return accent === "amber" ? "#d97706" : "#5B7092";
}

/** Format a Unix-seconds timestamp as a short weekday + time string. */
export function formatUnixShort(value: number | null | undefined): string {
  if (!value) return "-";
  return new Date(value * 1000).toLocaleString(undefined, {
    weekday: "short",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** Format an epoch-ms timestamp as a localized clock time (HH:MM:SS). */
export function formatClockTime(ms: number, lang: Lang): string {
  return new Date(ms).toLocaleTimeString(LOCALE[lang], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

/** Format a timezone offset in minutes as "UTC+09:00" / "UTC-03:30". */
export function formatTzOffset(min: number): string {
  const sign = min < 0 ? "-" : "+";
  const abs = Math.abs(min);
  const h = String(Math.floor(abs / 60)).padStart(2, "0");
  const m = String(abs % 60).padStart(2, "0");
  return `UTC${sign}${h}:${m}`;
}
