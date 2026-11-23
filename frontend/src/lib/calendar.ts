import { query } from "@solidjs/router";
import { api } from "./api";
import { addDays, dateLabel, daysBetween, monthOf, spanLabel, today, weekdayIndex } from "./dates";
import { valueOf } from "./records";

export type Unit = "day" | "week" | "month";

export function minutesBetween(start: string, end: string): number {
  return Math.round((Date.parse(end) - Date.parse(start)) / 60_000);
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
    title = spanLabel(at, addDays(end, -1));
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
