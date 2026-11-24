import { createAsync, query } from "@solidjs/router";
import { createEffect, createSignal, For, Match, onCleanup, onMount, Show, Switch } from "solid-js";
import TaskRow from "~/components/records/TaskRow";
import { Button, EmptyState, Icon, IconButton, Keyed, LinkButton, List, ListRow, Loading, PageHeader, Select, Tag } from "~/components/ui";
import { CalendarMode, type CalendarEvent } from "~/gen/famulen/v1/calendar_pb";
import type { Tile } from "~/gen/famulen/v1/settings_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { openSheet } from "~/lib/ask";
import { timeOn } from "~/lib/calendar";
import { addDays, dateLabel, dateText, daysLeft, nearness } from "~/lib/dates";
import * as icons from "~/lib/icons";
import { useWaiting } from "~/lib/inbox";
import { useOwner } from "~/lib/owner";
import { bucketOf, deadlinesOf, excerptOf, hrefOf, isOver, isPast, OWNER, type Rec, recordsQuery, SECRETARY, valueOf } from "~/lib/records";
import { keep, kept } from "~/lib/storage";
import { setView } from "~/lib/view";
import { workgroup } from "~/lib/workgroup";

const homeQuery = query(() => api.settings.getHome({}), "home");

const eventsQuery = query(async (workgroup: string, day: string) => {
  return api.calendar.listEvents({ from: day, to: addDays(day, 1), workgroup });
}, "home-events");

const workingQuery = query(async () => (await api.memory.getLayer({ layer: "working" })).working, "home-working");

/** How far ahead a deadline counts as near. */
const NEAR_DAYS = 30;

/** What things and contracts are due by `until`, past ones too, soonest first. */
const dueQuery = query(async (workgroup: string, until: string) => {
  const [things, contracts] = await Promise.all([
    api.records.listRecords({ kind: "holdings", workgroup }),
    api.records.listRecords({ kind: "contracts", workgroup }),
  ]);
  return [...things.records, ...contracts.records]
    .filter((r) => !isPast(r))
    .flatMap((r) => deadlinesOf(r).map((d) => ({ ...d, record: r })))
    .filter((d) => d.on <= until)
    .sort((a, b) => a.on.localeCompare(b.on));
}, "home-due");

/** The events of a day the owner has, as the tile shows them and the heading counts them. */
const ofDay = (events: CalendarEvent[]) => events.filter((e) => e.mode !== CalendarMode.HIDE && e.response !== "declined");

/** The tasks the secretary has in hand, as the tile shows them and the heading counts them. */
const inHand = (tasks: Rec[]) => tasks.filter((t) => !isOver(t) && valueOf(t, "Assignee") === SECRETARY);

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

const ARRANGING = "famulen.editing";

/** How many lines a tile shows before it leads to the page that has them all. */
const IN_TILE = 10;

function TodayTile() {
  const owner = useOwner();
  const day = owner.today;
  const events = createAsync(() => eventsQuery(workgroup(), day()));
  const shown = () => ofDay(events()?.events ?? []);
  const [now, setNow] = createSignal(0);
  onMount(() => {
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 60_000);
    onCleanup(() => clearInterval(timer));
  });
  const past = (e: CalendarEvent) => !e.allDay && now() > 0 && Date.parse(e.end) < now();
  const next = () => shown().find((e) => !e.allDay && now() > 0 && Date.parse(e.end) >= now());
  return (
    <Show when={events()} fallback={<Loading />}>
      <For each={shown()} fallback={<EmptyState>今日の予定はない</EmptyState>}>
        {(e) => (
          <div
            class="home-ev"
            classList={{ past: past(e), next: e === next(), faint: e.mode === CalendarMode.FAINT }}
            style={{ "--c": e.color || undefined }}
          >
            <time class="mono">{timeOn(e, day())}</time>
            <span class="item-main">
              <Show when={e.mode !== CalendarMode.FAINT} fallback={<span class="sub">予定あり</span>}>
                <a class="item-title item-pick" href={`/calendar?unit=day&at=${day()}&event=${encodeURIComponent(e.uid)}`}>
                  {e.status === "hold" ? "仮 · " : ""}
                  {e.title || "（題名なし）"}
                </a>
              </Show>
              <Show when={e.location || e.attendees.length > 0}>
                <span class="sub">
                  {[e.location, e.attendees.length > 0 ? `${e.attendees.length} 人` : ""].filter(Boolean).join(" · ")}
                </span>
              </Show>
            </span>
            <Show when={e.meeting && !past(e)}>
              <LinkButton small variant="go" icon={icons.video} href={e.meeting} class="home-join">
                入る
              </LinkButton>
            </Show>
          </div>
        )}
      </For>
    </Show>
  );
}

/** How many wait, and the way to the conversation, where they are answered. */
function ApprovalsHead() {
  const pending = useWaiting().approvals;
  return (
    <Show when={pending().length > 0}>
      <Tag tone="warn">{pending().length}</Tag>
      <Button small onClick={openSheet}>
        会話を開く
      </Button>
    </Show>
  );
}

function ApprovalsTile() {
  const pending = useWaiting().approvals;
  return (
    <For each={pending().slice(0, 3)} fallback={<EmptyState>いまはない</EmptyState>}>
      {(a) => (
        <div class="sub mono home-approval">
          #{a.id.toString()} {a.tool}
        </div>
      )}
    </For>
  );
}

const line = (r: Rec) => hrefOf(r.kind, r.id) ?? "/";

/** A line of a tile that leads to a record. */
function Line(props: { href: string; title: string; sub?: string; end?: string; tone?: string }) {
  return (
    <a class="home-line" href={props.href}>
      <span class="item-main">
        <span class="home-title">{props.title}</span>
        <Show when={props.sub}>
          <span class="sub home-sub">{props.sub}</span>
        </Show>
      </span>
      <Show when={props.end}>
        <span class={`rec-when ${props.tone ?? ""}`}>{props.end}</span>
      </Show>
    </a>
  );
}

function AideTile(props: { aide: string }) {
  const tasks = createAsync(() => recordsQuery("tasks", workgroup()));
  const working = createAsync(() => workingQuery());
  const open = () => inHand(tasks() ?? []);
  return (
    <Show when={open().length > 0 || (working() ?? []).length > 0} fallback={<EmptyState>{props.aide} が進めている仕事はない</EmptyState>}>
      <For each={working() ?? []}>{(w) => <Line href="/memory/working" title={w.title || w.name} end="作業中" />}</For>
      <For each={open()}>{(t) => <Line href={line(t)} title={t.title} sub={t.log.at(-1)?.text} />}</For>
    </Show>
  );
}

function TaskTile() {
  const owner = useOwner();
  const tasks = createAsync(() => recordsQuery("tasks", workgroup()));
  const soon = () =>
    (tasks() ?? [])
      .filter((t) => ["today", "week"].includes(bucketOf(t, owner.today())))
      .sort((a, b) => valueOf(a, "Due").localeCompare(valueOf(b, "Due")));
  return (
    <Show when={tasks()} fallback={<Loading />}>
      <Show when={soon().length > 0} fallback={<EmptyState>今日と今週のタスクはない</EmptyState>}>
        <div role="list">
          <Keyed each={soon().slice(0, IN_TILE)} by={(t) => t.id}>
            {(t) => <TaskRow task={t()} />}
          </Keyed>
        </div>
        <Show when={soon().length > IN_TILE}>
          <a class="home-more" href="/tasks">
            ほか {soon().length - IN_TILE} 件
          </a>
        </Show>
      </Show>
    </Show>
  );
}

function MemosTile() {
  const memos = createAsync(() => recordsQuery("memos", workgroup()));
  const latest = () =>
    (memos() ?? [])
      .filter((m) => !m.movedTo)
      .sort((a, b) => b.updated.localeCompare(a.updated))
      .slice(0, IN_TILE);
  return (
    <Show when={memos()} fallback={<Loading />}>
      <For each={latest()} fallback={<EmptyState>メモはまだない</EmptyState>}>
        {(m) => <Line href={line(m)} title={m.title} sub={excerptOf(m.notes, 80)} end={dateText(m.updated.slice(0, 10))} />}
      </For>
    </Show>
  );
}

function DueTile() {
  const owner = useOwner();
  const due = createAsync(() => dueQuery(workgroup(), addDays(owner.today(), NEAR_DAYS)));
  return (
    <Show when={due()} fallback={<Loading />}>
      <For each={due()} fallback={<EmptyState>{NEAR_DAYS} 日以内の期限はない</EmptyState>}>
        {(d) => (
          <Line
            href={line(d.record)}
            title={d.record.title}
            sub={d.what}
            end={daysLeft(d.on, owner.today())}
            tone={nearness(d.on, owner.today())}
          />
        )}
      </For>
    </Show>
  );
}

/** Where a tile's whole page is. */
const PAGES: { [kind: string]: { href: string; label: string } } = {
  today: { href: "/calendar", label: "カレンダーへ" },
  task: { href: "/tasks", label: "タスクへ" },
  memos: { href: "/memos", label: "メモへ" },
};

/** What is left to set up, until it is done or put away. */
function SetupGuide() {
  const home = createAsync(() => homeQuery());
  const steps = () => home()?.setup ?? [];
  return (
    <Show when={steps().some((s) => !s.done)}>
      <section class="card stack home-setup">
        <div class="row">
          <h2 class="grow">はじめに</h2>
          <Button variant="ghost" small onClick={() => void act(() => api.settings.hideSetup({}), "")}>
            あとで
          </Button>
        </div>
        <List>
          <For each={steps()}>
            {(s) => (
              <ListRow
                leads={!s.done}
                icon={s.done ? icons.check : undefined}
                href={s.path}
                title={<span classList={{ struck: s.done }}>{s.label}</span>}
              />
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
  const mine = () => (tasks() ?? []).filter((t) => (valueOf(t, "Assignee") || OWNER) === OWNER && bucketOf(t, day()) === "today").length;
  const tiles = () => home()?.tiles ?? [];
  const kinds = () => home()?.kinds ?? [];
  const label = (kind: string) => (kind === "aide" ? `${owner.aide()}が進めている` : (kinds().find((k) => k.kind === kind)?.label ?? kind));

  createEffect(() => setView("ホーム", dateLabel(day())));

  const [kind, setKind] = createSignal("");
  // A press is worked out from the tiles on screen, so the next one waits
  // until this one is saved and shown.
  const [saving, setSaving] = createSignal(false);
  const setTiles = async (next: TileShape[]) => {
    if (saving()) return;
    setSaving(true);
    try {
      await act(() => api.settings.setTiles({ tiles: next }), "");
    } finally {
      setSaving(false);
    }
  };
  const move = (i: number, op: Op) => setTiles(changed(tiles(), i, op));
  const addTile = () => {
    const k = kind() || kinds()[0]?.kind;
    if (k) void setTiles([...plain(tiles()), { kind: k, wide: true, tall: false }]);
  };

  // The arrange controls show only while arranging, and the choice survives a reload.
  const [arranging, setArranging] = createSignal(false);
  onMount(() => setArranging(kept(ARRANGING) === "1"));
  const toggleArranging = () => {
    setArranging(!arranging());
    keep(ARRANGING, arranging() ? "1" : undefined);
  };

  return (
    <>
      <PageHeader
        title={dateLabel(day())}
        tags={
          <span class="sub">
            予定 {ofDay(events()?.events ?? []).length} · あなたの今日のタスク {mine()} · {owner.aide()} が {inHand(tasks() ?? []).length}{" "}
            件を進めている
          </span>
        }
        actions={
          <Button variant={arranging() ? "primary" : "ghost"} small icon={icons.layoutGrid} aria-pressed={arranging()} onClick={toggleArranging}>
            並べ替える
          </Button>
        }
      />
      <Show when={arranging()}>
        <form
          class="row"
          onSubmit={(e) => {
            e.preventDefault();
            addTile();
          }}
        >
          <Select aria-label="足すタイル" onChange={(e) => setKind(e.currentTarget.value)}>
            <For each={kinds()}>
              {(k) => (
                <option value={k.kind} selected={k.kind === kind()}>
                  {k.label}
                </option>
              )}
            </For>
          </Select>
          <Button type="submit" icon={icons.plus} disabled={saving()}>
            タイルを足す
          </Button>
        </form>
      </Show>
      <SetupGuide />
      <div class="tiles">
        <For each={tiles()}>
          {(t, i) => (
            <section class="tile" classList={{ w2: t.wide, h2: t.tall }}>
              <header>
                <h2>{label(t.kind)}</h2>
                <Show
                  when={arranging()}
                  fallback={
                    <>
                      <Show when={t.kind === "approvals"}>
                        <ApprovalsHead />
                      </Show>
                      <Show when={PAGES[t.kind]}>
                        {(p) => (
                          <a class="tile-more" href={p().href} aria-label={p().label} title={p().label}>
                            <Icon svg={icons.chevronRight} size={16} />
                          </a>
                        )}
                      </Show>
                    </>
                  }
                >
                  <IconButton
                    plain
                    icon={icons.arrowUp}
                    label={`${label(t.kind)}を前へ`}
                    disabled={saving() || i() === 0}
                    onClick={() => void move(i(), "up")}
                  />
                  <IconButton
                    plain
                    icon={icons.arrowDown}
                    label={`${label(t.kind)}を後ろへ`}
                    disabled={saving() || i() === tiles().length - 1}
                    onClick={() => void move(i(), "down")}
                  />
                  <IconButton
                    plain
                    icon={icons.layoutGrid}
                    label={`${label(t.kind)}の大きさを変える`}
                    disabled={saving()}
                    onClick={() => void move(i(), "size")}
                  />
                  <IconButton
                    plain
                    icon={icons.x}
                    label={`${label(t.kind)}を外す`}
                    disabled={saving()}
                    onClick={() => void move(i(), "remove")}
                  />
                </Show>
              </header>
              <div class="tile-body">
                <Switch>
                  <Match when={t.kind === "today"}>
                    <TodayTile />
                  </Match>
                  <Match when={t.kind === "approvals"}>
                    <ApprovalsTile />
                  </Match>
                  <Match when={t.kind === "aide"}>
                    <AideTile aide={owner.aide()} />
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
