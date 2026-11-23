import { addDays, dateLabel, dateText, daysLeft, hm, nearness } from "~/lib/dates";
import { A, createAsync, query, useNavigate } from "@solidjs/router";
import { createEffect, createSignal, For, Match, onCleanup, onMount, Show, Switch } from "solid-js";
import { CalendarMode, type CalendarEvent } from "~/gen/famulen/v1/calendar_pb";
import TaskRow from "~/components/records/TaskRow";
import type { Tile } from "~/gen/famulen/v1/settings_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { openSheet } from "~/lib/ask";
import { workgroup } from "~/lib/workgroup";
import { useWaiting } from "~/lib/inbox";
import * as icons from "~/lib/icons";
import { bucketOf, deadlinesOf, hrefOf, excerptOf, isOver, isPast, OWNER, type Rec, recordsQuery, SECRETARY, valueOf } from "~/lib/records";
import { setView } from "~/lib/view";
import { useOwner } from "~/lib/owner";
import { Button, EmptyState, Icon, IconButton, LinkButton, List, Select } from "~/components/ui";

const homeQuery = query(() => api.settings.getHome({}), "home");

const eventsQuery = query(async (workgroup: string, day: string) => {
  return api.calendar.listEvents({ from: day, to: addDays(day, 1), workgroup });
}, "home-events");

const workingQuery = query(async () => (await api.memory.getLayer({ layer: "working" })).working, "home-working");

const dueQuery = query(async (workgroup: string) => {
  const [things, contracts] = await Promise.all([
    api.records.listRecords({ kind: "holdings", workgroup }),
    api.records.listRecords({ kind: "contracts", workgroup }),
  ]);
  return [...things.records, ...contracts.records]
    .filter((r) => !isPast(r))
    .flatMap((r) => deadlinesOf(r).map((d) => ({ ...d, record: r })))
    .sort((a, b) => a.on.localeCompare(b.on));
}, "home-due");

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
  const owner = useOwner();
  const day = owner.today;
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
        <EmptyState>今日の予定はない</EmptyState>
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
                <LinkButton variant="go" small icon={icons.video} href={e.meeting}>
入る
                </LinkButton>
              </Show>
            </div>
          );
        }}
      </For>
    </>
  );
}

function ApprovalsTile() {
  const pending = useWaiting().approvals;
  return (
    <Show when={pending().length > 0} fallback={<EmptyState>いまはない</EmptyState>}>
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
      <Button small onClick={openSheet}>
        会話を開く
      </Button>
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
      fallback={<EmptyState>{props.aide} が進めている仕事はない</EmptyState>}
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
  const owner = useOwner();
  const tasks = createAsync(() => recordsQuery("tasks", workgroup()));
  const soon = () => {
    const day = owner.today();
    return (tasks() ?? [])
      .filter((t) => ["today", "week"].includes(bucketOf(t, day)))
      .sort((a, b) => valueOf(a, "Due").localeCompare(valueOf(b, "Due")))
      .slice(0, 10);
  };
  return (
    <div class="rec-rows">
      <Show when={tasks() && soon().length === 0}>
        <EmptyState>今日と今週のタスクはない</EmptyState>
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
        <EmptyState>メモはまだない</EmptyState>
      </Show>
      <For each={latest()}>
        {(m) => (
          <A class="home-line" href={line(m)}>
            <span class="rec-main">
              <span>{m.title}</span>
              <span class="sub" style={{ "white-space": "nowrap", overflow: "hidden", "text-overflow": "ellipsis" }}>
                {excerptOf(m.notes, 80)}
              </span>
            </span>
            <span class="sub mono">{dateText(m.updated.slice(0, 10))}</span>
          </A>
        )}
      </For>
    </>
  );
}

function DueTile() {
  const owner = useOwner();
  const due = createAsync(() => dueQuery(workgroup()));
  const day = owner.today;
  return (
    <Show when={(due() ?? []).length > 0} fallback={<EmptyState>近い期限はない</EmptyState>}>
      <For each={(due() ?? []).slice(0, 6)}>
        {(d) => (
          <A class="home-line" href={line(d.record)}>
            <span class="rec-main">
              <span>{d.record.title}</span>
              <span class="sub">{d.what}</span>
            </span>
            <span class="rec-when" classList={{ late: nearness(d.on, day()) === "late" }}>
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
          <Button variant="ghost" small onClick={() => act(() => api.settings.hideSetup({}), "")}>
            あとで
          </Button>
        </div>
        <List>
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
        </List>
      </section>
    </Show>
  );
}

export default function Home() {
  const home = createAsync(() => homeQuery());
  const owner = useOwner();
  const day = owner.today;
  const events = createAsync(() => eventsQuery(workgroup(), day()));
  const tasks = createAsync(() => recordsQuery("tasks", workgroup()));
  const aide = owner.aide;
  const open = () => (tasks() ?? []).filter((t) => !isOver(t));
  const mine = () => open().filter((t) => (valueOf(t, "Assignee") || OWNER) === OWNER && bucketOf(t, day()) === "today").length;
  const aides = () => open().filter((t) => valueOf(t, "Assignee") === SECRETARY).length;
  const shownEvents = () => (events()?.events ?? []).filter((e) => e.mode === CalendarMode.SHOW).length;
  const tiles = () => home()?.tiles ?? [];
  const kinds = () => home()?.kinds ?? [];
  const label = (kind: string) =>
    kind === "aide" ? `${aide()}が進めている` : (kinds().find((k) => k.kind === kind)?.label ?? kind);

  createEffect(() => setView("ホーム", dateLabel(day())));

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
        <Button variant="ghost" small icon={icons.layoutGrid} style={{ "margin-left": "auto" }} onClick={toggleEditing}>
並べ替える
        </Button>
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
          <Select id="tile-kind" onChange={(e) => setKind(e.currentTarget.value)}>
            <For each={kinds()}>
              {(k) => (
                <option value={k.kind} selected={k.kind === kind()}>
                  {k.label}
                </option>
              )}
            </For>
          </Select>
          <Button icon={icons.plus} type="submit">
タイルを足す
          </Button>
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
                  <IconButton icon={icons.arrowUp} label="前へ" onClick={() => move(i(), "up")} />
                  <IconButton icon={icons.arrowDown} label="後ろへ" onClick={() => move(i(), "down")} />
                  <IconButton icon={icons.layoutGrid} label="大きさを変える" onClick={() => move(i(), "size")} />
                  <IconButton icon={icons.x} label="外す" onClick={() => move(i(), "remove")} />
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
