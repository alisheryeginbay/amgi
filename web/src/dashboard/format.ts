import { app } from "../lib/app.svelte";

export const DAY_SECONDS = 86_400;

export function number(n: number): string {
  return n.toLocaleString(app.environment.locale);
}

export function date(d: Date, options: Intl.DateTimeFormatOptions): string {
  return d.toLocaleDateString(app.environment.locale, options);
}

/** Row of a date in its week column, counting from the locale's first weekday. */
export function weekdayIndex(d: Date): number {
  return (d.getDay() - app.environment.firstWeekday + 7) % 7;
}
