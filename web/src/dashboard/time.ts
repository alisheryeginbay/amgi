/** Time spent studying, rounded to what matters at its size: "45 s", "23 min",
 *  "1 h 5 min". The app writes it the same way. */
export function studyTime(seconds: number): string {
  const secs = Math.round(seconds);
  if (secs < 60) return `${secs} s`;
  const minutes = Math.round(secs / 60);
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest === 0 ? `${hours} h` : `${hours} h ${rest} min`;
}

/** Seconds per review: a decimal under ten, "8.4 s", where whole seconds are too coarse. */
export function reviewTime(seconds: number): string {
  return seconds < 9.95 ? `${seconds.toFixed(1)} s` : studyTime(seconds);
}

/** Average time per day studied, over the days of `secs` that have any. */
export function dailyAverage(secs: number[]): number | null {
  const studied = secs.filter((s) => s > 0);
  return studied.length ? studied.reduce((a, b) => a + b, 0) / studied.length : null;
}
