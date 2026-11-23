import { query } from "@solidjs/router";
import { api } from "./api";
import { valueOf } from "./records";
import { today } from "./time";

export type Unit = "day" | "week" | "month";

export const WEEKDAYS = ["月", "火", "水", "木", "金", "土", "日"];

/*
 * Dates here are plain calendar days (YYYY-MM-DD) and times come from the
 * backend already in the owner's time zone, so they are read off the string
 * rather than through `Date`, which would bring in the browser's zone.
 */

function utc(d: string): Date {
  return new Date(`${d}T00:00:00Z`);
}

export function addDays(d: string, n: number): string {
  const t = utc(d);
  t.setUTCDate(t.getUTCDate() + n);
  return t.toISOString().slice(0, 10);
}

function daysBetween(from: string, to: string): number {
  return Math.round((utc(to).getTime() - utc(from).getTime()) / 86_400_000);
}

/** 0 for Monday. */
function weekdayIndex(d: string): number {
  return (utc(d).getUTCDay() + 6) % 7;
}

export function weekday(d: string): string {
  return WEEKDAYS[weekdayIndex(d)];
}

export function dayOf(d: string): number {
  return Number(d.slice(8, 10));
}

export function monthOf(d: string): number {
  return Number(d.slice(5, 7));
}

/** `11月22日（日）`. */
export function dateLabel(d: string): string {
  return `${monthOf(d)}月${dayOf(d)}日（${weekday(d)}）`;
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

export function minutesBetween(start: string, end: string): number {
  return Math.round((Date.parse(end) - Date.parse(start)) / 60_000);
}

/** `09:30` of a Unix time in `tz`. */
export function clock(at: bigint | number, tz: string): string {
  return new Intl.DateTimeFormat("ja-JP", {
    timeZone: tz || "UTC",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  }).format(new Date(Number(at) * 1000));
}

/** The hour it is now in `tz`. */
export function hourNow(tz: string): number {
  return Number(
    new Intl.DateTimeFormat("en-US", { timeZone: tz || "UTC", hour: "2-digit", hourCycle: "h23" }).format(new Date()),
  );
}

export type Range = {
  start: string;
  /** Exclusive. */
  end: string;
  days: string[];
  prev: string;
  next: string;
  title: string;
};

/** The days a unit shows around `at`, and where its arrows go. */
function rangeOf(unit: Unit, at: string): Range {
  let start = at;
  let end = addDays(at, 1);
  let prev = addDays(at, -1);
  let next = end;
  let title = dateLabel(at);
  if (unit === "week") {
    end = addDays(at, 7);
    prev = addDays(at, -7);
    next = end;
    title = `${monthOf(at)}月${dayOf(at)}日 – ${monthOf(addDays(end, -1))}月${dayOf(addDays(end, -1))}日`;
  } else if (unit === "month") {
    const first = `${at.slice(0, 8)}01`;
    const following = addDays(first, 32).slice(0, 8) + "01";
    const last = addDays(following, -1);
    start = addDays(first, -weekdayIndex(first));
    end = addDays(last, 7 - weekdayIndex(last));
    prev = addDays(first, -1).slice(0, 8) + "01";
    next = following;
    title = `${at.slice(0, 4)}年${monthOf(at)}月`;
  }
  const days = Array.from({ length: daysBetween(start, end) }, (_, i) => addDays(start, i));
  return { start, end, days, prev, next, title };
}

export function unitOf(v: string | undefined): Unit {
  return v === "day" || v === "month" ? v : "week";
}

/**
 * Side-by-side places for timed items that overlap: for each `[start, end]`,
 * its lane and how many lanes its group of overlapping items needs.
 */
export function lanes(spans: [number, number][]): [number, number][] {
  const order = spans.map((_, i) => i).sort((a, b) => spans[a][0] - spans[b][0] || spans[b][1] - spans[a][1]);
  const placed: [number, number][] = spans.map(() => [0, 1]);
  let group: number[] = [];
  let laneEnds: number[] = [];
  let groupEnd = Number.NEGATIVE_INFINITY;
  const close = () => {
    for (const i of group) placed[i][1] = Math.max(laneEnds.length, 1);
    group = [];
    laneEnds = [];
  };
  for (const i of order) {
    const [start, end] = spans[i];
    if (start >= groupEnd) close();
    let lane = laneEnds.findIndex((e) => e <= start);
    if (lane < 0) {
      laneEnds.push(end);
      lane = laneEnds.length - 1;
    } else {
      laneEnds[lane] = end;
    }
    placed[i][0] = lane;
    group.push(i);
    groupEnd = group.length === 1 ? end : Math.max(groupEnd, end);
  }
  close();
  return placed;
}

/** What the calendar pane shows: events and task due dates in a window. */
export const calendarQuery = query(async (workgroup: string, unit: Unit, at: string) => {
  const day = at || today((await api.settings.getPersona({})).timezone);
  const range = rangeOf(unit, day);
  const [listed, calendars, tasks] = await Promise.all([
    api.calendar.listEvents({ workgroup, from: range.start, to: range.end }),
    api.calendar.listCalendars({}),
    api.records.listRecords({ kind: "tasks", workgroup, openOnly: true }),
  ]);
  const dues = tasks.records
    .filter((t) => /^\d{4}-\d{2}-\d{2}/.test(valueOf(t, "Due")))
    .map((t) => ({ date: valueOf(t, "Due").slice(0, 10), title: t.title, id: t.id }));
  return {
    at: day,
    today: today(listed.timezone),
    tz: listed.timezone,
    range,
    events: listed.events,
    noCalendars: listed.noCalendars,
    calendars: calendars.calendars,
    dues,
  };
}, "calendar");

export const calendarsQuery = query(async () => (await api.calendar.listCalendars({})).calendars, "calendars");
