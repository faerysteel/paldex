const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });

const UNITS: ReadonlyArray<readonly [Intl.RelativeTimeFormatUnit, number]> = [
  ["year", 365 * 24 * 60 * 60 * 1000],
  ["month", 30 * 24 * 60 * 60 * 1000],
  ["day", 24 * 60 * 60 * 1000],
  ["hour", 60 * 60 * 1000],
  ["minute", 60 * 1000],
];

/**
 * Render an epoch-millisecond timestamp as e.g. "3 days ago".
 *
 * Anything under a minute reads as "just now" rather than counting seconds —
 * save timestamps are not precise enough for that to mean anything.
 */
export function relativeTime(ms: number | null): string {
  if (ms === null) return "never played";

  const delta = ms - Date.now();
  for (const [unit, msPerUnit] of UNITS) {
    if (Math.abs(delta) >= msPerUnit) {
      return rtf.format(Math.round(delta / msPerUnit), unit);
    }
  }
  return "just now";
}
