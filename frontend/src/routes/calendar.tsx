import { createAsync, useLocation, useNavigate } from "@solidjs/router";
import { createEffect, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import AddEvent from "~/components/records/AddEvent";
import EventPanel, { whenOf } from "~/components/records/EventPanel";
import { Button, Callout, IconButton, Legend, LinkButton, Loading, PageHeader, SegmentedLinks, SplitPane } from "~/components/ui";
import { CalendarMode, type CalendarEvent } from "~/gen/famulen/v1/calendar_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { calendarQuery, lanes, minutesOn, onDay, timeOn, type Unit, unitOf } from "~/lib/calendar";
import { dayOf, minutesNow, weekday } from "~/lib/dates";
import * as icons from "~/lib/icons";
import { useOwner } from "~/lib/owner";
import { useParam, withParams } from "~/lib/params";
import { isNarrow } from "~/lib/viewport";
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
/** Below this an event has one line. */
const SHORT_PX = 34;
/** How many events a day of the month shows before the rest are counted. */
const IN_MONTH = 4;

const faint = (e: CalendarEvent) => e.mode === CalendarMode.FAINT;
const hold = (e: CalendarEvent) => e.status === "hold";
const label = (e: CalendarEvent) => (faint(e) ? "予定あり" : `${hold(e) ? "仮 · " : ""}${e.title || "（題名なし）"}`);
/** Declined, or not answered yet: it stands back. */
const quiet = (e: CalendarEvent) => e.response === "declined" || e.response === "needs_action" || e.status === "tentative";
const looks = (e: CalendarEvent) => ({ faint: faint(e), hold: hold(e), quiet: quiet(e) });
const color = (e: CalendarEvent) => ({ "--c": !faint(e) && e.color ? e.color : undefined });

/** A task due on a day. */
function Due(props: { task: { id: string; title: string } }) {
  return (
    <a class="allday due" href={`/tasks?id=${props.task.id}`} title={`期日：${props.task.title}`}>
      {props.task.title}
    </a>
  );
}

/** An event as one line of a day. */
function Line(props: { event: CalendarEvent; day: string; time?: boolean; picked: boolean; onOpen: (e: CalendarEvent) => void }) {
  const e = () => props.event;
  return (
    <button
      type="button"
      class="allday"
      classList={{ ...looks(e()), picked: props.picked }}
      style={color(e())}
      title={label(e())}
      aria-pressed={props.picked}
      disabled={faint(e())}
      onClick={() => props.onOpen(e())}
    >
      <span>
        {props.time && !e().allDay ? `${timeOn(e(), props.day).split("–")[0]} ` : ""}
        {label(e())}
      </span>
    </button>
  );
}

export default function Calendar() {
  const navigate = useNavigate();
  const location = useLocation();
  const owner = useOwner();
  const unitParam = useParam("unit");
  const at = useParam("at");
  const uid = useParam("event");
  const record = useParam("record");
  const unit = () => unitOf(unitParam());
  const data = createAsync(() => calendarQuery(workgroup(), unit(), at()));
  const href = (u: Unit, d: string) => `/calendar?unit=${u}&at=${d}`;
  let scroller: HTMLDivElement | undefined;
  let scrolledFor = "";

  onMount(() => {
    if (!unitParam() && isNarrow()) navigate(withParams(`/calendar${location.search}`, { unit: "day" }), { replace: true });
  });

  const shown = () => (uid() ? data()?.events.find((e) => e.uid === uid()) : undefined);

  createEffect(() => {
    const d = data();
    if (!d) return;
    setView("カレンダー", UNITS.find(([u]) => u === unit())?.[1], d.range.title);
  });

  createEffect(() => {
    const d = data();
    const key = d ? `${unit()}:${d.range.start}` : "";
    if (!d || !scroller || key === scrolledFor) return;
    scrolledFor = key;
    const hour = d.range.days.includes(d.today) ? Math.max(Math.floor(minutesNow(d.tz) / 60) - 1, 0) : HOUR_SHOWN;
    const el = scroller;
    queueMicrotask(() => {
      const mark = el.querySelector<HTMLElement>(`[data-hour="${hour}"]`);
      const head = el.querySelector<HTMLElement>(".head");
      if (mark) el.scrollTop = mark.offsetTop - (head?.offsetHeight ?? 0);
    });
  });

  const [adding, setAdding] = createSignal("");
  const here = () => href(unit(), data()?.at ?? at());
  const open = (e: CalendarEvent) => {
    if (faint(e)) return;
    setSelection(`予定：${whenOf(e)} ${label(e)}`);
    navigate(`${here()}&event=${encodeURIComponent(e.uid)}`, { scroll: false });
  };
  const close = () => navigate(here(), { scroll: false });

  return (
    <Show when={data()} fallback={<Loading />}>
      {(d) => {
        const dues = (day: string) => d().dues.filter((t) => t.date === day);
        const on = (day: string) => d().events.filter((e) => onDay(e, day));
        const units = (): [string, string, boolean][] => UNITS.map(([u, l]) => [href(u, d().at), l, u === unit()]);
        const add = () => setAdding(d().range.days.includes(d().today) && unit() !== "day" ? d().today : d().at);
        const line = (e: CalendarEvent, day: string, time = false) => <Line event={e} day={day} time={time} picked={e.uid === uid()} onOpen={open} />;
        return (
          <>
            <SplitPane
              name="calendar"
              floats
              open={!!shown() || !!record()}
              label={shown()?.title || "予定"}
              onClose={close}
              bar={
                <>
                  <LinkButton small class="split-back" icon={icons.chevronLeft} href={here()}>
                    カレンダー
                  </LinkButton>
                  <span class="grow" />
                  <IconButton class="split-close" variant="ghost" icon={icons.x} label="予定を閉じる" onClick={close} />
                </>
              }
              detail={<EventPanel event={shown()} record={record()} back={here()} />}
            >
              <PageHeader
                title={d().range.title}
                actions={
                  <>
                    <SegmentedLinks class="d-only" label="表示の単位" items={units()} />
                    <Button variant="primary" icon={icons.plus} class="d-only" onClick={add}>
                      予定を足す
                    </Button>
                  </>
                }
                tags={
                  <span class="row">
                    <LinkButton small href={href(unit(), d().today)}>
                      今日
                    </LinkButton>
                    <LinkButton small icon={icons.chevronLeft} label="前へ" href={href(unit(), d().range.prev)} />
                    <LinkButton small icon={icons.chevronRight} label="次へ" href={href(unit(), d().range.next)} />
                    <IconButton
                      variant="ghost"
                      icon={icons.refresh}
                      label="カレンダーを読み直す"
                      onClick={() => void act(() => api.calendar.refreshCalendars({}), "読み直している")}
                    />
                  </span>
                }
              />
              <Show when={d().noCalendars}>
                <Callout>
                  カレンダーがまだない。設定のサービスで Google のアカウントを開き、<a href="/settings/services">カレンダーの節で足す</a>と、ここに出る。
                </Callout>
              </Show>
              <For each={d().calendars.filter((c) => c.error)}>{(c) => <Callout tone="warn">{c.name} を読めなかった。前に読めた予定を出している。</Callout>}</For>
              <Show
                when={unit() !== "month"}
                fallback={
                  <div class="month">
                    <For each={d().range.days}>
                      {(day) => (
                        <div class="month-day" classList={{ today: day === d().today, outside: day.slice(0, 7) !== d().at.slice(0, 7) }}>
                          <a class="month-date" href={href("day", day)} aria-label={`${dayOf(day)}日（${weekday(day)}）を開く`}>
                            {dayOf(day)} <span>{weekday(day)}</span>
                          </a>
                          <For each={dues(day)}>{(t) => <Due task={t} />}</For>
                          <For each={on(day).slice(0, IN_MONTH)}>{(e) => line(e, day, true)}</For>
                          <Show when={on(day).length > IN_MONTH}>
                            <a class="sub" href={href("day", day)}>
                              ほか {on(day).length - IN_MONTH} 件
                            </a>
                          </Show>
                        </div>
                      )}
                    </For>
                  </div>
                }
              >
                <div class="hours-scroll" ref={scroller}>
                  <div class="week" style={{ "--days": d().range.days.length }}>
                    <span class="week-corner" />
                    <For each={d().range.days}>
                      {(day) => (
                        <div class="head" classList={{ today: day === d().today }}>
                          <a class="week-date" href={href("day", day)} aria-label={`${dayOf(day)}日（${weekday(day)}）を開く`}>
                            {weekday(day)} <b>{dayOf(day)}</b>
                          </a>
                          <For each={dues(day)}>{(t) => <Due task={t} />}</For>
                          <For each={on(day).filter((e) => e.allDay)}>{(e) => line(e, day)}</For>
                        </div>
                      )}
                    </For>
                    <div class="hours">
                      <For each={HOURS}>{(h) => <span data-hour={h}>{h}:00</span>}</For>
                    </div>
                    <For each={d().range.days}>
                      {(day) => (
                        <DayColumn day={day} events={on(day).filter((e) => !e.allDay)} picked={uid()} onOpen={open} now={day === d().today ? d().tz : ""} />
                      )}
                    </For>
                  </div>
                </div>
              </Show>
              <Legend
                class="d-only"
                items={[
                  ...d().calendars.map((c) => ({ text: c.name, style: { background: c.color } })),
                  { text: "ワークグループの外（予定あり）", style: { background: "var(--faint-event)" } },
                  { text: `${owner.aide()}の仮押さえ`, style: { border: "1.5px dashed var(--workgroup)" } },
                ]}
              />
            </SplitPane>
            <Show when={!shown() && !record()}>
              <div class="m-float">
                <SegmentedLinks class="m-toggles" label="表示の単位" items={units()} />
                <IconButton plain class="m-fab" size={28} icon={icons.plus} label="予定を足す" onClick={add} />
              </div>
            </Show>
            <AddEvent day={adding()} open={!!adding()} onOpenChange={(o) => !o && setAdding("")} />
          </>
        );
      }}
    </Show>
  );
}

/** The line where it is now, moving with the clock. */
function Now(props: { tz: string }) {
  const [at, setAt] = createSignal(0);
  onMount(() => {
    setAt(minutesNow(props.tz));
    const timer = setInterval(() => setAt(minutesNow(props.tz)), 60_000);
    onCleanup(() => clearInterval(timer));
  });
  return <div class="ev-now" style={{ top: `${(at() * HOUR_PX) / 60}px` }} aria-hidden="true" />;
}

/** The part of each event that falls on one day, side by side where they overlap. */
function DayColumn(props: { day: string; events: CalendarEvent[]; picked: string; onOpen: (e: CalendarEvent) => void; now: string }) {
  const spans = () =>
    props.events.map((e): [number, number] => {
      const [from, to] = minutesOn(e, props.day);
      return [from, Math.max(to, from + MIN_MINUTES)];
    });
  const placed = () => lanes(spans());
  return (
    <div class="col" style={{ height: `${24 * HOUR_PX}px` }}>
      <For each={props.events}>
        {(e, i) => {
          const top = () => ((spans()[i()]?.[0] ?? 0) * HOUR_PX) / 60;
          const height = () => Math.max((((spans()[i()]?.[1] ?? 0) - (spans()[i()]?.[0] ?? 0)) * HOUR_PX) / 60 - 3, 16);
          const lane = () => placed()[i()] ?? [0, 1];
          const picked = () => e.uid === props.picked;
          return (
            <button
              type="button"
              class="ev"
              classList={{ ...looks(e), short: height() < SHORT_PX, picked: picked() }}
              title={`${timeOn(e, props.day)} ${label(e)}`}
              aria-pressed={picked()}
              disabled={faint(e)}
              style={{
                ...color(e),
                top: `${top()}px`,
                height: `${height()}px`,
                left: `calc(${lane()[0]} * 100% / ${lane()[1]} + 2px)`,
                width: `calc(100% / ${lane()[1]} - 4px)`,
              }}
              onClick={() => props.onOpen(e)}
            >
              <b>{label(e)}</b>
              <span>
                {timeOn(e, props.day)}
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
