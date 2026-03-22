/**
 * Shared formatting utilities for dates and relative times.
 */

/**
 * Format a date as a relative time string (e.g., "Today", "3 days ago")
 * or fall back to a localized date string for older dates.
 */
export function formatRelativeTime(date: Date | null): string {
  if (!date) return "—";

  const now = new Date();
  const diff = now.getTime() - date.getTime();
  const days = Math.floor(diff / (1000 * 60 * 60 * 24));

  if (days === 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days < 7) return `${days} days ago`;
  if (days < 30) return `${Math.floor(days / 7)} weeks ago`;
  if (days < 365) return `${Math.floor(days / 30)} months ago`;

  return formatDate(date);
}

/**
 * Format a date as a localized short date string (e.g., "Mar 22, 2026").
 */
export function formatDate(date: Date | null): string {
  if (!date) return "—";

  return date.toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}
