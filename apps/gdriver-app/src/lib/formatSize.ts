import i18next from "i18next";

/**
 * Format a byte count as a human-readable file size string using the browser's
 * Intl.NumberFormat for locale-aware number formatting.
 *
 * @param bytes - Size in bytes. Returns "" for 0 when `allowZero` is false (default for optional usage).
 * @param options.precision - Decimal places for KB/MB/GB (default: 1)
 * @param options.allowZero - If false (default), returns "" when bytes is 0 or falsy
 */
export function formatSize(
  bytes?: number,
  options?: { precision?: number; allowZero?: boolean },
): string {
  if (!bytes && !options?.allowZero) return "";

  const b = bytes ?? 0;
  const precision = options?.precision ?? 1;
  const locale = i18next.language || "en";

  if (b === 0) return new Intl.NumberFormat(locale).format(0) + " KB";

  const units = ["B", "KB", "MB", "GB"];
  const i = Math.min(Math.floor(Math.log(b) / Math.log(1024)), units.length - 1);
  const value = b / Math.pow(1024, i);

  const formatted = new Intl.NumberFormat(locale, {
    minimumFractionDigits: i === 0 ? 0 : precision,
    maximumFractionDigits: i === 0 ? 0 : precision,
  }).format(value);

  return `${formatted} ${units[i]}`;
}
