import i18next from "i18next";

/**
 * Format a timestamp as a relative time string ("Just now", "5 minutes ago", etc.).
 * Accepts either a Unix timestamp (number) or a Date object.
 */
export function formatRelativeTime(ts: number | Date): string {
  const t = i18next.t;
  const diffMs = Date.now() - (typeof ts === "number" ? ts : ts.getTime());
  const diffMin = Math.floor(diffMs / 60_000);

  if (diffMin < 1) return t("time.just_now");
  if (diffMin < 60) return t("time.minutes_ago", { count: diffMin });

  const diffHrs = Math.floor(diffMin / 60);
  if (diffHrs < 24) return t("time.hours_ago", { count: diffHrs });

  const diffDays = Math.floor(diffHrs / 24);
  return t("time.days_ago", { count: diffDays });
}

/**
 * Format a timestamp as a synced relative time string ("Synced 5 minutes ago", etc.).
 * Accepts either a Unix timestamp (number) or a Date object.
 */
export function formatSyncedTime(ts: number | Date): string {
  const t = i18next.t;
  const diffMs = Date.now() - (typeof ts === "number" ? ts : ts.getTime());
  const diffMin = Math.floor(diffMs / 60_000);

  if (diffMin < 1) return t("time.just_now");
  if (diffMin < 60) return t("time.synced_ago", { count: diffMin });

  const diffHrs = Math.floor(diffMin / 60);
  if (diffHrs < 24) return t("time.synced_hours_ago", { count: diffHrs });

  const diffDays = Math.floor(diffHrs / 24);
  return t("time.synced_days_ago", { count: diffDays });
}
