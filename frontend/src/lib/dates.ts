/*
 * How days and times are read, counted and written, in one place.
 *
 * Days are plain calendar days (YYYY-MM-DD) and the backend's times of events
 * come already in the owner's time zone, so both are read off the string
 * rather than through `Date`, which would bring in the browser's zone. Unix
 * times are put in the owner's zone, which the caller passes.
 *
 * There are two ways to write one: in words (`11月22日 09:30`), and short for
 * a column of times (`11/22 09:30`).
 */

const FROM_MONDAY = ["月", "火", "水", "木", "金", "土", "日"];

/** The weekdays as cron and `Date` number them, from Sunday. */
export const WEEKDAYS = ["日", "月", "火", "水", "木", "金", "土"];

function utc(d: string): Date {
  return new Date(`${d}T00:00:00Z`);
}

export function addDays(d: string, n: number): string {
  const t = utc(d);
  t.setUTCDate(t.getUTCDate() + n);
  return t.toISOString().slice(0, 10);
}

/** How many days `to` is after `from`; NaN when either is not a day. */
export function daysBetween(from: string, to: string): number {
  return Math.round((utc(to).getTime() - utc(from).getTime()) / 86_400_000);
}

/** 0 for Monday. */
export function weekdayIndex(d: string): number {
  return (utc(d).getUTCDay() + 6) % 7;
}

export function weekday(d: string): string {
  return FROM_MONDAY[weekdayIndex(d)];
}

export function dayOf(d: string): number {
  return Number(d.slice(8, 10));
}

export function monthOf(d: string): number {
  return Number(d.slice(5, 7));
}

/** The local date of an RFC 3339 time written in the owner's zone. */
export function localDate(at: string): string {
  return at.slice(0, 10);
}

/** `09:30` of an RFC 3339 time written in the owner's zone. */
export function hm(at: string): string {
  return at.slice(11, 16);
}

export function minutesOfDay(at: string): number {
  return Number(at.slice(11, 13)) * 60 + Number(at.slice(14, 16));
}

function parts(at: bigint | number, tz: string) {
  const all = new Intl.DateTimeFormat("ja-JP", {
    timeZone: tz || "UTC",
    year: "numeric",
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  }).formatToParts(new Date(Number(at) * 1000));
  const get = (t: string) => all.find((p) => p.type === t)?.value ?? "";
  return { year: get("year"), month: get("month"), day: get("day"), time: `${get("hour")}:${get("minute")}` };
}

/** Today as YYYY-MM-DD in `tz`. */
export function today(tz: string): string {
  return dayIn(Math.floor(Date.now() / 1000), tz);
}

/** `09:30` of a Unix time in `tz`. */
export function clock(at: bigint | number, tz: string): string {
  return parts(at, tz).time;
}

/** Minutes since midnight in `tz`, now. */
export function minutesNow(tz: string): number {
  const time = clock(Math.floor(Date.now() / 1000), tz);
  return Number(time.slice(0, 2)) * 60 + Number(time.slice(3, 5));
}

/** `11/22 09:30` in `tz`, for a column of times. */
export function shortTime(at: bigint | number, tz: string): string {
  const p = parts(at, tz);
  return `${p.month}/${p.day} ${p.time}`;
}

const thisYear = () => String(new Date().getFullYear());

/** `11月22日 09:30` in `tz`, with its year when it is not this one. */
export function dateTimeText(at: bigint | number, tz: string): string {
  const p = parts(at, tz);
  return `${p.year === thisYear() ? "" : `${p.year}年`}${p.month}月${p.day}日 ${p.time}`;
}

/** The day a Unix time falls on in `tz`. */
export function dayIn(at: bigint | number, tz: string): string {
  return new Intl.DateTimeFormat("sv-SE", { timeZone: tz || "UTC" }).format(new Date(Number(at) * 1000));
}

/** `11月22日（日）` of a Unix time in `tz`. */
export function dayLabel(at: bigint | number, tz: string): string {
  return dateLabel(dayIn(at, tz));
}

/** `11月22日（日）`. */
export function dateLabel(d: string): string {
  return `${monthOf(d)}月${dayOf(d)}日（${weekday(d)}）`;
}

/** `11月22日 – 11月28日`; the last day is included. */
export function spanLabel(from: string, to: string): string {
  return `${monthOf(from)}月${dayOf(from)}日 – ${monthOf(to)}月${dayOf(to)}日`;
}

/** `3月1日` of a written date, with its year when it is not this one and its time when it has one. */
export function dateText(text: string): string {
  const m = /^(\d{4})-(\d{2})-(\d{2})(?:[ T](\d{2}:\d{2}))?/.exec(text.trim());
  if (!m) return text;
  const year = m[1] === thisYear() ? "" : `${m[1]}年`;
  return `${year}${Number(m[2])}月${Number(m[3])}日${m[4] ? ` ${m[4]}` : ""}`;
}

/** `3 日後`, `今日`, `5 日過ぎた`. */
export function daysLeft(on: string, today: string): string {
  const days = daysBetween(today, on.slice(0, 10));
  if (Number.isNaN(days)) return "";
  if (days === 0) return "今日";
  if (days === 1) return "明日";
  return days > 0 ? `${days} 日後` : `${-days} 日過ぎた`;
}

/** How near a day is: past, today, or still to come. */
export function nearness(on: string, today: string): "late" | "soon" | "later" {
  const days = daysBetween(today, on.slice(0, 10));
  return days < 0 ? "late" : days === 0 ? "soon" : "later";
}
