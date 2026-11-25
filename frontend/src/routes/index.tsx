import { createAsync, query } from "@solidjs/router";
import { createEffect, createMemo, createSignal, For, Index, type JSX, Match, on, onCleanup, onMount, Show, Switch } from "solid-js";
import { ProgressLine } from "~/components/records/RunProgress";
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
import { below, COLUMNS, inOrder, movedTo, type Place, type Placed, settled, sizedTo } from "~/lib/tiles";
import { setView } from "~/lib/view";
import { type DelegatedTask, delegatedQuery, stageOf, standingText } from "~/lib/workers";
import { workgroup } from "~/lib/workgroup";

const homeQuery = query(() => api.settings.getHome({}), "home");

const eventsQuery = query(async (workgroup: string, day: string) => {
  return api.calendar.listEvents({ from: day, to: addDays(day, 1), workgroup });
}, "home-events");

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

/** The tasks that are the secretary's and not over. */
const inHand = (tasks: Rec[]) => tasks.filter((t) => !isOver(t) && valueOf(t, "Assignee") === SECRETARY);

/** A tile as it is saved, without what the API's messages carry beside it. */
const plain = (t: Tile): Placed => ({ kind: t.kind, x: t.x, y: t.y, width: t.width, height: t.height });

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
        <div class="home-approval">
          {a.deed} <span class="sub">{a.place}</span>
        </div>
      )}
    </For>
  );
}

const line = (r: Rec) => hrefOf(r.kind, r.id) ?? "/";

/** A line of a tile that leads to a record. */
function Line(props: { href: string; title: string; sub?: JSX.Element; end?: string; tone?: string }) {
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

/** A part of a tile, under what its lines have in common; not there when it has none. */
function Group(props: { title: string; tone: "now" | "wait" | "next"; count: number; children: JSX.Element }) {
  return (
    <Show when={props.count > 0}>
      <div class="home-group" data-tone={props.tone}>
        <h3>
          <span class="dot" />
          {props.title} {props.count}
        </h3>
        {props.children}
      </div>
    </Show>
  );
}

function AideTile(props: { aide: string }) {
  const tasks = createAsync(() => recordsQuery("tasks", workgroup()));
  const delegated = createAsync(() => delegatedQuery({ workgroup: workgroup() }));
  const mine = (status: string) => inHand(tasks() ?? []).filter((t) => (valueOf(t, "Status") || "pending") === status);
  const handed = (stage: string) => (delegated()?.queue ?? []).filter((t) => stageOf(t) === stage);
  const now = () => mine("in_progress").length + handed("running").length;
  const wait = () => mine("waiting").length + handed("owner").length;
  const later = () => [...handed("soon"), ...handed("next"), ...handed("held")];
  const next = () => mine("pending").length + later().length;
  const handedLine = (t: DelegatedTask) => <Line href={`/tasks?tab=delegated&id=${t.task}`} title={t.title} sub={t.run?.percent || t.run?.doing ? <ProgressLine run={t.run} /> : "委託"} end={standingText(t)} />;
  return (
    <Show when={now() + wait() + next() > 0} fallback={<EmptyState>{props.aide} が進めている仕事はない</EmptyState>}>
      <Group title="動いている" tone="now" count={now()}>
        <For each={handed("running")}>{handedLine}</For>
        <For each={mine("in_progress")}>{(t) => <Line href={line(t)} title={t.title} sub={t.log.at(-1)?.text} />}</For>
      </Group>
      <Group title="待っている" tone="wait" count={wait()}>
        <For each={handed("owner")}>{handedLine}</For>
        <For each={mine("waiting")}>{(t) => <Line href={line(t)} title={t.title} sub={valueOf(t, "Waiting for")} />}</For>
      </Group>
      <Group title="これから" tone="next" count={next()}>
        <For each={later()}>{handedLine}</For>
        <For each={mine("pending")}>{(t) => <Line href={line(t)} title={t.title} sub={t.log.at(-1)?.text} />}</For>
      </Group>
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

/** How tall a cell of the grid is, and what is between two. */
const ROW = 44;
const GAP = 12;

const ICONS: { [kind: string]: string } = {
  today: icons.calendar,
  approvals: icons.shieldCheck,
  aide: icons.activity,
  task: icons.listCheck,
  memos: icons.notes,
  due: icons.hourglass,
};

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
  const delegated = createAsync(() => delegatedQuery({ workgroup: workgroup() }));
  const going = () =>
    inHand(tasks() ?? []).filter((t) => valueOf(t, "Status") === "in_progress").length +
    (delegated()?.queue ?? []).filter((t) => stageOf(t) === "running").length;
  const tiles = () => home()?.tiles ?? [];
  const kinds = () => home()?.kinds ?? [];
  const label = (kind: string) => (kind === "aide" ? `${owner.aide()}が進めている` : (kinds().find((k) => k.kind === kind)?.label ?? kind));

  createEffect(() => setView("ホーム", dateLabel(day())));

  // The arrange controls show only while arranging, and the choice survives a reload.
  const [arranging, setArranging] = createSignal(false);
  const [kind, setKind] = createSignal("");
  // What is changed is shown before it is saved. A change made while another
  // is being saved is saved after it, the last one of them.
  const [draft, setDraft] = createSignal<Placed[]>();
  const [saving, setSaving] = createSignal(false);
  let waiting: Placed[] | undefined;
  // While they are arranged the tiles keep their order in the list, so that the
  // one held or focused stays the same element wherever it is put.
  const shown = createMemo(() => {
    const there = draft() ?? settled(tiles().map(plain));
    return arranging() ? there : inOrder(there);
  });
  const setTiles = async (next: Placed[]) => {
    setDraft(next);
    waiting = next;
    if (saving()) return;
    setSaving(true);
    while (waiting) {
      const tiles: Placed[] = waiting;
      waiting = undefined;
      const saved = await act(() => api.settings.setTiles({ tiles }), "");
      if (!saved && !waiting && !held) setDraft(undefined);
    }
    setSaving(false);
  };
  // What was read before the change still shows for a moment after it is
  // saved, so the draft stands until what is read is the change.
  createEffect(
    on([home, saving], ([read, busy]) => {
      const next = draft();
      if (!next || busy || held) return;
      const same = JSON.stringify(next) === JSON.stringify((read?.tiles ?? []).map(plain));
      if (same) setDraft(undefined);
    }),
  );
  const placed = (i: number, place: Place) => settled(shown().map((t, n) => (n === i ? { ...t, ...place } : t)), i);
  const remove = (i: number) => setTiles(shown().filter((_, n) => n !== i));
  /** Where each tile comes in the order they are read. */
  const ranks = () => {
    const order = inOrder(shown());
    return shown().map((t) => order.indexOf(t));
  };
  /** In one column, a tile and the one before or after it change places. */
  const swap = (i: number, by: -1 | 1) => {
    const all = shown();
    const other = ranks().indexOf(ranks()[i] + by);
    const [a, b] = [all[i], all[other]];
    if (!a || !b) return;
    void setTiles(all.map((t, n) => (n === i ? { ...b, kind: a.kind } : n === other ? { ...a, kind: b.kind } : t)));
  };
  const addTile = () => {
    const k = kind() || kinds()[0]?.kind;
    if (k) void setTiles([...shown(), { kind: k, x: 0, y: below(shown()), width: 6, height: 6 }]);
  };

  let grid: HTMLDivElement | undefined;
  const cell = () => {
    const box = grid?.getBoundingClientRect();
    return { left: box?.left ?? 0, top: box?.top ?? 0, width: ((box?.width ?? 0) + GAP) / COLUMNS, height: ROW + GAP };
  };
  /** A tile held by its head, or by its corner, until it is let go. */
  let held: { i: number; from: Placed[]; place: (x: number, y: number) => Place } | undefined;
  const hold = (i: number, how: "move" | "size") => (e: PointerEvent & { currentTarget: HTMLElement }) => {
    if (!arranging() || e.button !== 0 || (how === "move" && e.target instanceof Element && e.target.closest("button:not(.tile-grip)"))) return;
    if (grid && getComputedStyle(grid).gridTemplateColumns.split(" ").length < COLUMNS) return;
    e.preventDefault();
    e.currentTarget.setPointerCapture(e.pointerId);
    const from = shown();
    const tile = from[i];
    if (!tile) return;
    const c = cell();
    const [dx, dy] = [e.clientX - c.left - tile.x * c.width, e.clientY - c.top - tile.y * c.height];
    held = {
      i,
      from,
      place: (x, y) => {
        const now = cell();
        return how === "move"
          ? movedTo(tile, Math.round((x - now.left - dx) / now.width), Math.round((y - now.top - dy) / now.height))
          : sizedTo(
              tile,
              Math.round((x - now.left) / now.width) - tile.x,
              Math.round((y - now.top) / now.height) - tile.y,
            );
      },
    };
    setDragged(i);
  };
  const [dragged, setDragged] = createSignal(-1);
  const drag = (e: PointerEvent) => {
    if (!held) return;
    const { i, from, place } = held;
    const at = place(e.clientX, e.clientY);
    setDraft(settled(from.map((t, n) => (n === i ? { ...t, ...at } : t)), i));
  };
  const drop = (kept: boolean) => {
    if (!held) return;
    held = undefined;
    setDragged(-1);
    const next = draft();
    if (kept && next) void setTiles(next);
    else setDraft(undefined);
  };
  const onKey = (i: number, how: "move" | "size") => (e: KeyboardEvent) => {
    const by = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] }[e.key];
    const tile = shown()[i];
    if (!by || !tile) return;
    e.preventDefault();
    const at = how === "move" ? movedTo(tile, tile.x + by[0], tile.y + by[1]) : sizedTo(tile, tile.width + by[0], tile.height + by[1]);
    void setTiles(placed(i, at));
  };

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
            予定 {ofDay(events()?.events ?? []).length} · あなたの今日のタスク {mine()} · {owner.aide()} が {going()}{" "}
            件を進めている
          </span>
        }
        actions={
          <Button variant={arranging() ? "primary" : "ghost"} small icon={icons.layoutGrid} aria-pressed={arranging()} onClick={toggleArranging}>
            配置を変える
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
          <Button type="submit" icon={icons.plus}>
            タイルを足す
          </Button>
        </form>
      </Show>
      <SetupGuide />
      <div
        class="tiles"
        classList={{ arranging: arranging(), dragging: dragged() >= 0 }}
        ref={grid}
        style={{ "--row": `${ROW}px`, "--gap": `${GAP}px`, "--rows": below(shown()) + (arranging() ? 4 : 0) }}
        onPointerMove={drag}
        onPointerUp={() => drop(true)}
        onPointerCancel={() => drop(false)}
      >
        <Index each={shown()}>
          {(t, i) => (
            <section
              class="tile"
              classList={{ held: dragged() === i }}
              style={{ "grid-column": `${t().x + 1} / span ${t().width}`, "grid-row": `${t().y + 1} / span ${t().height}`, "--rank": ranks()[i] }}
            >
              <header onPointerDown={hold(i, "move")}>
                <Show when={arranging()} fallback={<Icon svg={ICONS[t().kind] ?? icons.layoutGrid} size={16} />}>
                  <button type="button" class="icon-plain tile-grip" aria-label={`${label(t().kind)}を動かす`} title="つかんで動かす（矢印キーでも動く）" onKeyDown={onKey(i, "move")}>
                    <Icon svg={icons.gripVertical} size={16} />
                  </button>
                </Show>
                <h2>{label(t().kind)}</h2>
                <Show
                  when={arranging()}
                  fallback={
                    <>
                      <Show when={t().kind === "approvals"}>
                        <ApprovalsHead />
                      </Show>
                      <Show when={PAGES[t().kind]}>
                        {(p) => (
                          <a class="tile-more" href={p().href} aria-label={p().label} title={p().label}>
                            <Icon svg={icons.chevronRight} size={16} />
                          </a>
                        )}
                      </Show>
                    </>
                  }
                >
                  <span class="tile-order">
                    <IconButton plain icon={icons.arrowUp} label={`${label(t().kind)}を前へ`} disabled={ranks()[i] === 0} onClick={() => swap(i, -1)} />
                    <IconButton plain icon={icons.arrowDown} label={`${label(t().kind)}を後ろへ`} disabled={ranks()[i] === shown().length - 1} onClick={() => swap(i, 1)} />
                  </span>
                  <IconButton plain icon={icons.x} label={`${label(t().kind)}を外す`} onClick={() => void remove(i)} />
                </Show>
              </header>
              <div class="tile-body">
                <Switch>
                  <Match when={t().kind === "today"}>
                    <TodayTile />
                  </Match>
                  <Match when={t().kind === "approvals"}>
                    <ApprovalsTile />
                  </Match>
                  <Match when={t().kind === "aide"}>
                    <AideTile aide={owner.aide()} />
                  </Match>
                  <Match when={t().kind === "task"}>
                    <TaskTile />
                  </Match>
                  <Match when={t().kind === "memos"}>
                    <MemosTile />
                  </Match>
                  <Match when={t().kind === "due"}>
                    <DueTile />
                  </Match>
                </Switch>
              </div>
              <Show when={arranging()}>
                <button type="button" class="tile-corner" aria-label={`${label(t().kind)}の大きさを変える`} title="つかんで大きさを変える（矢印キーでも変わる）" onPointerDown={hold(i, "size")} onKeyDown={onKey(i, "size")} />
              </Show>
            </section>
          )}
        </Index>
      </div>
    </>
  );
}
