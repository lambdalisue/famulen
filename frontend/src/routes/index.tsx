import { A, createAsync, query, useNavigate } from "@solidjs/router";
import { createEffect, createSignal, For, Match, onCleanup, onMount, Show, Switch } from "solid-js";
import Icon from "~/components/Icon";
import { CalendarMode, type CalendarEvent } from "~/gen/famulen/v1/calendar_pb";
import TaskRow from "~/components/records/TaskRow";
import type { Tile } from "~/gen/famulen/v1/settings_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { openSheet } from "~/lib/ask";
import { workgroup } from "~/lib/workgroup";
import { inboxQuery, personaQuery } from "~/lib/data";
import * as icons from "~/lib/icons";
import { bucketOf, daysLeft, deadlinesOf, hrefOf, isOver, OWNER, plainText, type Rec, recordsQuery, SECRETARY, valueOf } from "~/lib/records";
import { today } from "~/lib/time";
import { setView } from "~/lib/view";
import "~/components/records/records.css";

const homeQuery = query(() => api.settings.getHome({}), "home");

const eventsQuery = query(async (workgroup: string, day: string) => {
  const next = new Date(`${day}T00:00:00Z`);
  next.setUTCDate(next.getUTCDate() + 1);
  return api.calendar.listEvents({ from: day, to: next.toISOString().slice(0, 10), workgroup });
}, "home-events");

const workingQuery = query(async () => (await api.memory.getLayer({ layer: "working" })).working, "home-working");

const dueQuery = query(async (workgroup: string) => {
  const [things, contracts] = await Promise.all([
    api.records.listRecords({ kind: "holdings", workgroup }),
    api.records.listRecords({ kind: "contracts", workgroup }),
  ]);
  return [...things.records, ...contracts.records]
    .filter((r) => !["released", "ended"].includes(valueOf(r, "Status")))
    .flatMap((r) => deadlinesOf(r).map((d) => ({ ...d, record: r })))
    .sort((a, b) => a.on.localeCompare(b.on));
}, "home-due");

const WEEKDAYS = ["日", "月", "火", "水", "木", "金", "土"];

/** `11月22日（日）` from YYYY-MM-DD. */
function dateLabel(day: string): string {
  const d = new Date(`${day}T00:00:00Z`);
  return `${d.getUTCMonth() + 1}月${d.getUTCDate()}日（${WEEKDAYS[d.getUTCDay()]}）`;
}

/** HH:MM of an RFC 3339 time already in the owner's time zone. */
function hm(at: string): string {
  return at.slice(11, 16);
}

type Op = "up" | "down" | "size" | "remove";
type TileShape = { kind: string; wide: boolean; tall: boolean };

function plain(tiles: Tile[]): TileShape[] {
  return tiles.map((t) => ({ kind: t.kind, wide: t.wide, tall: t.tall }));
}

function changed(tiles: Tile[], i: number, op: Op): TileShape[] {
  const next = plain(tiles);
  if (i < 0 || i >= next.length) return next;
  switch (op) {
    case "remove":
      next.splice(i, 1);
      break;
    case "up":
      if (i > 0) [next[i - 1], next[i]] = [next[i], next[i - 1]];
      break;
    case "down":
      if (i + 1 < next.length) [next[i], next[i + 1]] = [next[i + 1], next[i]];
      break;
    case "size": {
      const t = next[i];
      [t.wide, t.tall] = !t.wide && !t.tall ? [true, false] : t.wide && !t.tall ? [true, true] : [false, false];
      break;
    }
  }
  return next;
}

const EDITING = "famulen.editing";

function TodayTile() {
  const persona = createAsync(() => personaQuery());
  const day = () => today(persona()?.timezone ?? "UTC");
  const events = createAsync(() => eventsQuery(workgroup(), day()));
  const shown = () => (events()?.events ?? []).filter((e) => e.mode !== CalendarMode.HIDE && e.response !== "declined");
  const [now, setNow] = createSignal(0);
  onMount(() => {
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 60_000);
    onCleanup(() => clearInterval(timer));
  });
  const past = (e: CalendarEvent) => now() > 0 && Date.parse(e.end) < now();
  const next = () => shown().find((e) => !e.allDay && now() > 0 && Date.parse(e.end) >= now());
  return (
    <>
      <Show when={events() && shown().length === 0}>
        <p class="empty">今日の予定はない</p>
      </Show>
      <For each={shown()}>
        {(e) => {
          const faint = () => e.mode === CalendarMode.FAINT;
          return (
            <div class="home-ev" classList={{ past: past(e), next: e === next() }} style={{ "--c": e.color || "#0F6E68" }}>
              <span class="bar-line" />
              <time class="mono">{e.allDay ? "終日" : `${hm(e.start)}–${hm(e.end)}`}</time>
              <span class="rec-main">
                <Show when={!faint()} fallback={<span class="sub">予定あり</span>}>
                  <A
                    href={`/calendar?unit=day&at=${day()}&event=${encodeURIComponent(e.uid)}`}
                    style={{ color: "inherit", "text-decoration": "none", "font-size": "14px", "font-weight": 600 }}
                  >
                    {e.status === "hold" ? "仮 · " : ""}
                    {e.title}
                  </A>
                </Show>
                <Show when={e.location || e.attendees.length > 0}>
                  <span class="sub">
                    {e.location}
                    {e.location && e.attendees.length > 0 ? " · " : ""}
                    {e.attendees.length > 0 ? `${e.attendees.length} 人` : ""}
                  </span>
                </Show>
              </span>
              <Show when={e.meeting && !past(e)}>
                <a class="btn small go" href={e.meeting} target="_blank" rel="noreferrer">
                  <Icon svg={icons.video} size={13} />
                  入る
                </a>
              </Show>
            </div>
          );
        }}
      </For>
    </>
  );
}

function ApprovalsTile() {
  const inbox = createAsync(() => inboxQuery());
  const pending = () => (inbox()?.approvals ?? []).filter((a) => a.status === "pending");
  return (
    <Show when={pending().length > 0} fallback={<p class="empty">いまはない</p>}>
      <div class="row">
        <span class="mono" style={{ "font-size": "24px" }}>
          {pending().length}
        </span>
        <span class="sub">件 · 会話の中のカードで答える</span>
      </div>
      <For each={pending().slice(0, 3)}>
        {(a) => (
          <div class="sub mono">
            #{a.id.toString()} {a.tool}
          </div>
        )}
      </For>
      <button type="button" class="btn small" onClick={openSheet}>
        会話を開く
      </button>
    </Show>
  );
}

const line = (r: Rec) => hrefOf(r.kind, r.id) ?? "/";

function AideTile(props: { aide: string }) {
  const tasks = createAsync(() => recordsQuery("tasks", workgroup()));
  const working = createAsync(() => workingQuery());
  const open = () => (tasks() ?? []).filter((t) => !isOver(t) && valueOf(t, "Assignee") === SECRETARY);
  return (
    <Show
      when={open().length > 0 || (working() ?? []).length > 0}
      fallback={<p class="empty">{props.aide} が進めている仕事はない</p>}
    >
      <For each={open().slice(0, 6)}>
        {(t) => (
          <A class="home-line" href={line(t)}>
            <Icon svg={icons.activity} size={14} />
            <span class="rec-main">
              <span>{t.title}</span>
              <Show when={t.log.at(-1)}>{(l) => <span class="sub">{l().text}</span>}</Show>
            </span>
          </A>
        )}
      </For>
      <For each={(working() ?? []).slice(0, 3)}>{(w) => <div class="home-line sub">作業中 · {w.name}</div>}</For>
    </Show>
  );
}

function TaskTile() {
  const navigate = useNavigate();
  const persona = createAsync(() => personaQuery());
  const tasks = createAsync(() => recordsQuery("tasks", workgroup()));
  const soon = () => {
    const day = today(persona()?.timezone ?? "UTC");
    return (tasks() ?? [])
      .filter((t) => ["today", "week"].includes(bucketOf(t, day)))
      .sort((a, b) => valueOf(a, "Due").localeCompare(valueOf(b, "Due")))
      .slice(0, 10);
  };
  return (
    <div class="rec-rows">
      <Show when={tasks() && soon().length === 0}>
        <p class="empty">今日と今週のタスクはない</p>
      </Show>
      <For each={soon()}>
        {(t) => (
          <div
            class="rec-row"
            onClick={(e) => {
              if (!(e.target instanceof Element && e.target.closest("a,button"))) navigate(line(t));
            }}
          >
            <TaskRow task={t} />
          </div>
        )}
      </For>
    </div>
  );
}

function MemosTile() {
  const memos = createAsync(() => recordsQuery("memos", workgroup()));
  const latest = () => [...(memos() ?? [])].sort((a, b) => b.updated.localeCompare(a.updated)).slice(0, 6);
  return (
    <>
      <Show when={memos()?.length === 0}>
        <p class="empty">メモはまだない</p>
      </Show>
      <For each={latest()}>
        {(m) => (
          <A class="home-line" href={line(m)}>
            <span class="rec-main">
              <span>{m.title}</span>
              <span class="sub" style={{ "white-space": "nowrap", overflow: "hidden", "text-overflow": "ellipsis" }}>
                {plainText(m.notes).replace(/[#>*`|-]+/g, " ").slice(0, 80)}
              </span>
            </span>
            <span class="sub mono">{m.updated.slice(5, 10)}</span>
          </A>
        )}
      </For>
    </>
  );
}

function DueTile() {
  const persona = createAsync(() => personaQuery());
  const due = createAsync(() => dueQuery(workgroup()));
  const day = () => today(persona()?.timezone ?? "UTC");
  return (
    <Show when={(due() ?? []).length > 0} fallback={<p class="empty">近い期限はない</p>}>
      <For each={(due() ?? []).slice(0, 6)}>
        {(d) => (
          <A class="home-line" href={line(d.record)}>
            <span class="rec-main">
              <span>{d.record.title}</span>
              <span class="sub">{d.what}</span>
            </span>
            <span class="rec-when" classList={{ late: d.on < day() }}>
              {daysLeft(d.on, day())}
            </span>
          </A>
        )}
      </For>
    </Show>
  );
}

/** What is left to set up, until it is done or put away. */
function SetupGuide() {
  const home = createAsync(() => homeQuery());
  const steps = () => home()?.setup ?? [];
  return (
    <Show when={steps().some((s) => !s.done)}>
      <section
        class="card"
        style={{ display: "flex", "flex-direction": "column", gap: "8px", "border-color": "var(--workgroup)" }}
      >
        <div class="row">
          <h2 class="grow">はじめに</h2>
          <button type="button" class="btn small ghost" onClick={() => act(() => api.settings.hideSetup({}), "")}>
            あとで
          </button>
        </div>
        <div class="list">
          <For each={steps()}>
            {(s) => (
              <A class="item" href={s.path} style={{ color: "inherit", "text-decoration": "none" }}>
                <Icon svg={s.done ? icons.check : icons.chevronRight} size={16} />
                <span class="grow" style={s.done ? { color: "var(--muted)", "text-decoration": "line-through" } : {}}>
                  {s.label}
                </span>
              </A>
            )}
          </For>
        </div>
      </section>
    </Show>
  );
}

export default function Home() {
  const home = createAsync(() => homeQuery());
  const persona = createAsync(() => personaQuery());
  const day = () => today(persona()?.timezone ?? "UTC");
  const events = createAsync(() => eventsQuery(workgroup(), day()));
  const tasks = createAsync(() => recordsQuery("tasks", workgroup()));
  const aide = () => persona()?.name || "秘書";
  const open = () => (tasks() ?? []).filter((t) => !isOver(t));
  const mine = () => open().filter((t) => (valueOf(t, "Assignee") || OWNER) === OWNER && bucketOf(t, day()) === "today").length;
  const aides = () => open().filter((t) => valueOf(t, "Assignee") === SECRETARY).length;
  const shownEvents = () => (events()?.events ?? []).filter((e) => e.mode === CalendarMode.SHOW).length;
  const tiles = () => home()?.tiles ?? [];
  const kinds = () => home()?.kinds ?? [];
  const label = (kind: string) =>
    kind === "aide" ? `${aide()}が進めている` : (kinds().find((k) => k.kind === kind)?.label ?? kind);

  setView("ホーム");
  createEffect(() => setView(`ホーム · ${dateLabel(day())}`));

  const [kind, setKind] = createSignal("");
  const setTiles = (next: TileShape[]) => act(() => api.settings.setTiles({ tiles: next }), "");
  const move = (i: number, op: Op) => setTiles(changed(tiles(), i, op));
  const addTile = () => {
    const k = kind() || kinds()[0]?.kind;
    if (!k) return;
    void setTiles([...plain(tiles()), { kind: k, wide: true, tall: false }]);
  };

  // The arrange controls show only while arranging, and the choice survives a reload.
  onMount(() => {
    try {
      if (localStorage.getItem(EDITING) === "1") document.body.classList.add("editing");
    } catch {
      // Storage may be unavailable; arranging just starts off.
    }
  });
  const toggleEditing = () => {
    const on = document.body.classList.toggle("editing");
    try {
      localStorage.setItem(EDITING, on ? "1" : "0");
    } catch {
      // Storage may be unavailable; the choice lasts until the page is left.
    }
  };

  return (
    <>
      <div class="row" style={{ "align-items": "baseline", gap: "12px" }}>
        <h1>{dateLabel(day())}</h1>
        <span class="sub">
          予定 {shownEvents()} · あなたの今日のタスク {mine()} · {aide()} が {aides()} 件を進めている
        </span>
        <button type="button" class="btn small ghost" style={{ "margin-left": "auto" }} onClick={toggleEditing}>
          <Icon svg={icons.layoutGrid} size={14} />
          並べ替える
        </button>
        <form
          class="row tile-ctl"
          onSubmit={(e) => {
            e.preventDefault();
            addTile();
          }}
        >
          <label for="tile-kind" class="sr">
            足すタイル
          </label>
          <select id="tile-kind" class="input" onChange={(e) => setKind(e.currentTarget.value)}>
            <For each={kinds()}>
              {(k) => (
                <option value={k.kind} selected={k.kind === kind()}>
                  {k.label}
                </option>
              )}
            </For>
          </select>
          <button type="submit" class="btn">
            <Icon svg={icons.plus} size={14} />
            タイルを足す
          </button>
        </form>
      </div>
      <SetupGuide />
      <div class="grid">
        <For each={tiles()}>
          {(t, i) => (
            <section class={`tile${t.wide ? " w2" : ""}${t.tall ? " h2" : ""}`}>
              <header>
                <h2 class="grow" style={{ "font-size": "13px" }}>
                  {label(t.kind)}
                </h2>
                <div class="row tile-ctl" style={{ gap: "2px" }}>
                  <button type="button" class="btn small" aria-label="前へ" onClick={() => move(i(), "up")}>
                    <Icon svg={icons.arrowUp} size={12} />
                  </button>
                  <button type="button" class="btn small" aria-label="後ろへ" onClick={() => move(i(), "down")}>
                    <Icon svg={icons.arrowDown} size={12} />
                  </button>
                  <button type="button" class="btn small" aria-label="大きさを変える" onClick={() => move(i(), "size")}>
                    <Icon svg={icons.layoutGrid} size={12} />
                  </button>
                  <button type="button" class="btn small" aria-label="外す" onClick={() => move(i(), "remove")}>
                    <Icon svg={icons.x} size={12} />
                  </button>
                </div>
              </header>
              <div style={{ "overflow-y": "auto" }}>
                <Switch>
                  <Match when={t.kind === "today"}>
                    <TodayTile />
                  </Match>
                  <Match when={t.kind === "approvals"}>
                    <ApprovalsTile />
                  </Match>
                  <Match when={t.kind === "aide"}>
                    <AideTile aide={aide()} />
                  </Match>
                  <Match when={t.kind === "task"}>
                    <TaskTile />
                  </Match>
                  <Match when={t.kind === "memos"}>
                    <MemosTile />
                  </Match>
                  <Match when={t.kind === "due"}>
                    <DueTile />
                  </Match>
                </Switch>
              </div>
            </section>
          )}
        </For>
      </div>
    </>
  );
}
