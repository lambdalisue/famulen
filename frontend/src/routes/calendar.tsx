import { createAsync, useNavigate, useSearchParams } from "@solidjs/router";
import { createEffect, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import Icon from "~/components/Icon";
import AddEvent from "~/components/records/AddEvent";
import EventSheet, { whenOf } from "~/components/records/EventSheet";
import { CalendarMode, type CalendarEvent } from "~/gen/famulen/v1/calendar_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import {
  calendarQuery,
  dayOf,
  hm,
  hourNow,
  lanes,
  localDate,
  minutesBetween,
  minutesOfDay,
  type Unit,
  unitOf,
  weekday,
} from "~/lib/calendar";
import { personaQuery } from "~/lib/data";
import * as icons from "~/lib/icons";
import { setSelection, setView } from "~/lib/view";
import { workgroup } from "~/lib/workgroup";

const UNITS: [Unit, string][] = [
  ["day", "日"],
  ["week", "週"],
  ["month", "月"],
];
const HOURS = Array.from({ length: 24 }, (_, h) => h);
/** Where the day and week grids open when today is not shown: the working hours. */
const HOUR_SHOWN = 7;
const HOUR_PX = 48;
const MIN_MINUTES = 20;
/** How many events a day of the month shows before the rest are counted. */
const IN_MONTH = 4;

function one(v: string | string[] | undefined): string | undefined {
  return Array.isArray(v) ? v[0] : v;
}

const faint = (e: CalendarEvent) => e.mode === CalendarMode.FAINT;
const hold = (e: CalendarEvent) => e.status === "hold";
const label = (e: CalendarEvent) => (faint(e) ? "予定あり" : `${hold(e) ? "仮 · " : ""}${e.title || "（題名なし）"}`);
/** Declined, or not answered yet: it stands back. */
const quiet = (e: CalendarEvent) => e.response === "declined" || e.response === "needs_action" || e.status === "tentative";

function place(start: string, end: string) {
  const length = Math.max(minutesBetween(start, end), MIN_MINUTES);
  const top = Math.max((minutesOfDay(start) * HOUR_PX) / 60, 0);
  const height = Math.max((length * HOUR_PX) / 60 - 3, 16);
  return { top, height };
}

function laneStyle(top: number, height: number, lane: number, n: number) {
  return `top:${top}px;height:${height}px;left:calc(${lane} * 100% / ${n} + 2px);width:calc(100% / ${n} - 4px)`;
}

export default function Calendar() {
  const [params] = useSearchParams();
  const navigate = useNavigate();
  const unit = () => unitOf(one(params.unit));
  const at = () => one(params.at) ?? "";
  const data = createAsync(() => calendarQuery(workgroup(), unit(), at()));
  const persona = createAsync(() => personaQuery());
  const aide = () => persona()?.name || "秘書";
  const href = (u: Unit, d: string) => `/calendar?unit=${u}&at=${d}`;
  let scroller: HTMLDivElement | undefined;
  let scrolledFor = "";

  onMount(() => {
    if (one(params.unit) === undefined && window.matchMedia("(max-width: 700px)").matches) {
      navigate(`/calendar?unit=day${at() ? `&at=${at()}` : ""}`, { replace: true });
    }
  });

  createEffect(() => {
    const d = data();
    if (!d) return;
    setView(`カレンダー · ${UNITS.find(([u]) => u === unit())?.[1] ?? "週"} · ${d.range.title}`);
  });

  createEffect(() => {
    const d = data();
    const key = d ? `${unit()}:${d.range.start}` : "";
    if (!d || !scroller || key === scrolledFor) return;
    scrolledFor = key;
    const hour = d.range.days.includes(d.today) ? Math.max(hourNow(d.tz) - 1, 0) : HOUR_SHOWN;
    const el = scroller;
    queueMicrotask(() => {
      const mark = el.querySelector<HTMLElement>(`[data-hour="${hour}"]`);
      const head = el.querySelector<HTMLElement>(".head");
      if (mark) el.scrollTop = mark.offsetTop - (head?.offsetHeight ?? 0);
    });
  });

  const onDay = (e: CalendarEvent, d: string) => {
    const s = localDate(e.start);
    return s === d || (e.allDay && s <= d && localDate(e.end) > d);
  };

  const [adding, setAdding] = createSignal("");
  const shown = () => {
    const uid = one(params.event);
    return uid ? data()?.events.find((e) => e.uid === uid) : undefined;
  };
  const here = () => href(unit(), data()?.at ?? at());
  const open = (e: CalendarEvent) => {
    if (faint(e)) return;
    setSelection(`予定：${whenOf(e)} ${label(e)}`);
    navigate(`${here()}&event=${encodeURIComponent(e.uid)}`, { scroll: false });
  };

  return (
    <Show when={data()}>
      {(d) => {
        const dues = (day: string) => d().dues.filter((t) => t.date === day);
        const timedOn = (day: string) => d().events.filter((e) => !e.allDay && localDate(e.start) === day);
        return (
          <>
            <div class="row">
              <a class="btn" href={href(unit(), d().today)}>
                今日
              </a>
              <a class="btn small" href={href(unit(), d().range.prev)} aria-label="前へ">
                <Icon svg={icons.chevronLeft} size={16} />
              </a>
              <a class="btn small" href={href(unit(), d().range.next)} aria-label="次へ">
                <Icon svg={icons.chevronRight} size={16} />
              </a>
              <h1 style={{ "margin-left": "6px", "font-size": "18px" }}>{d().range.title}</h1>
              <div class="seg d-only" style={{ "margin-left": "auto" }}>
                <For each={UNITS}>
                  {([u, l]) => (
                    <a class={u === unit() ? "on" : ""} href={href(u, d().at)}>
                      {l}
                    </a>
                  )}
                </For>
              </div>
              <button type="button" class="btn primary d-only" onClick={() => setAdding(d().range.days.includes(d().today) ? d().today : d().at)}>
                <Icon svg={icons.plus} size={14} />
                予定を足す
              </button>
              <button
                type="button"
                class="btn small"
                aria-label="読み直す"
                onClick={() => act(() => api.calendar.refreshCalendars({}), "読み直している")}
              >
                <Icon svg={icons.refresh} size={14} />
              </button>
            </div>
            <Show when={d().noCalendars}>
              <div class="notice">
                カレンダーがまだない。設定のサービスで Google のアカウントを開き、<a href="/settings/services">カレンダーの節で足す</a>と、ここに出る。
              </div>
            </Show>
            <For each={d().calendars.filter((c) => c.error)}>
              {(c) => <div class="notice">{c.name} を読めなかった。前に読めた予定を出している。</div>}
            </For>
            <Show
              when={unit() !== "month"}
              fallback={
                <div class="scroll">
                  <div style={{ display: "grid", "grid-template-columns": "repeat(7,minmax(0,1fr))" }}>
                    <For each={d().range.days}>
                      {(day) => (
                        <div style={{ "min-height": "96px", border: "1px solid var(--line2)", padding: "4px" }}>
                          <div
                            class="sub"
                            style={day === d().today ? { color: "var(--workgroup)", "font-weight": 700 } : undefined}
                          >
                            {dayOf(day)} {weekday(day)}
                          </div>
                          <For each={dues(day)}>{(t) => <a class="allday due" href={`/tasks?id=${t.id}`}>{t.title}</a>}</For>
                          <For each={d().events.filter((e) => onDay(e, day)).slice(0, IN_MONTH)}>
                            {(e) => (
                              <button
                                type="button"
                                class="allday"
                                classList={{ hold: hold(e), faint: faint(e), quiet: quiet(e) }}
                                style={{ "border-left": `3px solid ${e.color || "#0F6E68"}` }}
                                onClick={() => open(e)}
                              >
                                {e.allDay ? "" : `${hm(e.start)} `}
                                {label(e)}
                              </button>
                            )}
                          </For>
                          <Show when={d().events.filter((e) => onDay(e, day)).length > IN_MONTH}>
                            <a class="sub" href={href("day", day)}>
                              ほか {d().events.filter((e) => onDay(e, day)).length - IN_MONTH} 件
                            </a>
                          </Show>
                        </div>
                      )}
                    </For>
                  </div>
                </div>
              }
            >
              <div class="scroll" data-scroll-to={HOUR_SHOWN} ref={scroller}>
                <div class="week" style={{ "grid-template-columns": `52px repeat(${d().range.days.length}, minmax(0,1fr))` }}>
                  <span />
                  <For each={d().range.days}>
                    {(day) => (
                      <div class="head" style={day === d().today ? { background: "var(--accent-soft)" } : undefined}>
                        <span>{weekday(day)}</span>
                        <b>{dayOf(day)}</b>
                        <For each={dues(day)}>{(t) => <a class="allday due" href={`/tasks?id=${t.id}`}>{t.title}</a>}</For>
                        <For each={d().events.filter((e) => e.allDay && onDay(e, day))}>
                          {(e) => (
                            <button
                              type="button"
                              class="allday"
                              classList={{ hold: hold(e), faint: faint(e), quiet: quiet(e) }}
                              style={{ "border-left": `3px solid ${e.color || "#0F6E68"}` }}
                              onClick={() => open(e)}
                            >
                              {label(e)}
                            </button>
                          )}
                        </For>
                      </div>
                    )}
                  </For>
                  <div class="hours">
                    <For each={HOURS}>{(h) => <span data-hour={h}>{h}:00</span>}</For>
                  </div>
                  <For each={d().range.days}>
                    {(day) => <DayColumn events={timedOn(day)} onOpen={open} now={day === d().today ? d().tz : ""} />}
                  </For>
                </div>
              </div>
            </Show>
            <div class="m-float">
              <nav class="seg m-toggles" aria-label="表示の単位">
                <For each={UNITS}>
                  {([u, l]) => (
                    <a class={u === unit() ? "on" : ""} href={href(u, d().at)} aria-pressed={u === unit()}>
                      {l}
                    </a>
                  )}
                </For>
              </nav>
              <button type="button" class="m-fab" aria-label="予定を足す" onClick={() => setAdding(d().at)}>
                <Icon svg={icons.plus} size={28} />
              </button>
            </div>
            <div class="legend">
              <For each={d().calendars}>
                {(c) => (
                  <span>
                    <i style={{ background: c.color }} />
                    {c.name}
                  </span>
                )}
              </For>
              <span>
                <i style={{ background: "repeating-linear-gradient(135deg,#F4F3EE 0,#F4F3EE 3px,#ECEAE3 3px,#ECEAE3 6px)" }} />
                ワークグループの外（予定あり）
              </span>
              <span>
                <i style={{ border: "1.5px dashed var(--workgroup)", background: "#fff" }} />
                {aide()}の仮押さえ
              </span>
            </div>
            <AddEvent day={adding()} open={!!adding()} onOpenChange={(o) => !o && setAdding("")} />
            <EventSheet event={shown()} record={one(params.record) ?? ""} back={here()} onClose={() => navigate(here(), { scroll: false })} />
          </>
        );
      }}
    </Show>
  );
}

/** The line where it is now, moving with the clock. */
function Now(props: { tz: string }) {
  const minutes = () => {
    const t = new Intl.DateTimeFormat("ja-JP", { timeZone: props.tz, hour: "2-digit", minute: "2-digit", hourCycle: "h23" }).format(new Date());
    return Number(t.slice(0, 2)) * 60 + Number(t.slice(3, 5));
  };
  const [at, setAt] = createSignal(0);
  onMount(() => {
    setAt(minutes());
    const timer = setInterval(() => setAt(minutes()), 60_000);
    onCleanup(() => clearInterval(timer));
  });
  return <div class="ev-now" style={{ top: `${(at() * HOUR_PX) / 60}px` }} aria-hidden="true" />;
}

function DayColumn(props: { events: CalendarEvent[]; onOpen: (e: CalendarEvent) => void; now: string }) {
  const placed = () =>
    lanes(
      props.events.map((x) => {
        const s = Date.parse(x.start);
        return [s, Math.max(Date.parse(x.end), s + MIN_MINUTES * 60_000)];
      }),
    );
  return (
    <div class="col" style={{ height: `${24 * HOUR_PX}px` }}>
      <For each={props.events}>
        {(e, i) => {
          const box = () => place(e.start, e.end);
          const lane = () => placed()[i()] ?? [0, 1];
          const color = () => (!faint(e) && e.color ? `;border-left-color:${e.color}` : "");
          return (
            <button
              type="button"
              class={`ev${faint(e) ? " faint" : ""}${hold(e) ? " hold" : ""}${quiet(e) ? " quiet" : ""}${box().height < 34 ? " short" : ""}`}
              title={`${hm(e.start)}–${hm(e.end)} ${label(e)}`}
              style={laneStyle(box().top, box().height, lane()[0], lane()[1]) + color()}
              onClick={() => props.onOpen(e)}
            >
              <b>{label(e)}</b>
              <span>
                {hm(e.start)}–{hm(e.end)}
                {e.location ? ` · ${e.location}` : ""}
              </span>
            </button>
          );
        }}
      </For>
      <Show when={props.now}>{(tz) => <Now tz={tz()} />}</Show>
    </div>
  );
}
