import { createAsync } from "@solidjs/router";
import { createMemo, For, type JSX, Show } from "solid-js";
import {
  Button,
  Disclosure,
  EmptyState,
  Icon,
  IconButton,
  LinkButton,
  List,
  ListRow,
  Loading,
  PageHeader,
  Section,
  Tag,
  whenConfirmed,
} from "~/components/ui";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { dateText, shortTime } from "~/lib/dates";
import * as icons from "~/lib/icons";
import { useOwner } from "~/lib/owner";
import { useParam, withParams } from "~/lib/params";
import { hrefOf, isOver, type Rec, recordsQuery, valueOf, WORKER } from "~/lib/records";
import {
  type DelegatedTask,
  delegatedQuery,
  outcomeLook,
  outcomeText,
  placeOf,
  runHref,
  runnerText,
  stageLook,
  stageOf,
  STAGES,
  standingLook,
  standingText,
  type WorkerRun,
} from "~/lib/workers";
import { workgroup } from "~/lib/workgroup";
import RecordsFrame from "./RecordsFrame";
import RunAsk from "./RunAsk";
import { ProgressLine, RunSteps } from "./RunProgress";

const retry = (id: string) => act(() => api.workers.retryDelegated({ id }), "並びに戻した");
const cancel = (id: string, title: string) =>
  whenConfirmed(
    `「${title}」の委託をやめる`,
    "委託をやめる",
    () => act(() => api.workers.cancelDelegated({ id }), "やめた"),
    "まだ始まっていなければ始まらず、手代が動いていれば止まる。",
  );

/** Whether a task has a run under way that can be watched. */
const isSeen = (t: DelegatedTask) => !!t.run && ["running", "waiting", "unreachable"].includes(t.state);

function Watch(props: { run: WorkerRun; title: string }) {
  return (
    <LinkButton small href={runHref(props.run)} label={`「${props.title}」の出力を見る`}>
      出力
    </LinkButton>
  );
}

function Standing(props: { of: DelegatedTask }) {
  const look = () => standingLook(props.of);
  return (
    <Tag tone={look().tone} icon={look().icon}>
      {standingText(props.of)}
    </Tag>
  );
}

/** What a delegated task that may not start yet waits for. */
function Awaits(props: { of: DelegatedTask; task: Rec | undefined; class?: string }) {
  return (
    <>
      <Show when={props.of.state === "blocked" && (props.task?.blockers.length ?? 0) > 0}>
        <span class={props.class}>
          <For each={props.task?.blockers}>
            {(b, i) => (
              <>
                {i() > 0 ? "、" : ""}
                <a href={hrefOf(b.kind, b.id)}>{b.title}</a>
              </>
            )}
          </For>
          の完了を待っている
        </span>
      </Show>
      <Show when={props.of.state === "later" && props.task}>
        {(task) => <span class={props.class}>{dateText(valueOf(task(), "Start"))} から</span>}
      </Show>
    </>
  );
}

/** One delegated task in the queue: how it stands, with what, and why it does not go on. */
function QueueRow(props: { of: DelegatedTask; href: string; current: boolean; place: number; task: Rec | undefined }) {
  const t = () => props.of;
  return (
    <ListRow
      record={t().task}
      class="delegated-row"
      href={props.href}
      current={props.current}
      lead={<span class="delegated-number mono">{props.place || ""}</span>}
      title={t().title}
      sub={
        <>
          <Standing of={t()} />
          <span>{runnerText(t().runner)}</span>
          <Show when={(t().run?.attempt ?? 0) > 1}>
            <span>{t().run?.attempt} 回目</span>
          </Show>
          <Awaits of={t()} task={props.task} />
          <Show when={t().why}>
            <span>{t().why}</span>
          </Show>
          <Show when={isSeen(t())}>
            <ProgressLine run={t().run!} />
          </Show>
        </>
      }
      end={
        <>
          <Show when={isSeen(t())}>
            <Watch run={t().run!} title={t().title} />
          </Show>
          <Show when={t().state === "still"}>
            <Button small onClick={() => void retry(t().task)}>
              並びに戻す
            </Button>
          </Show>
          <IconButton plain icon={icons.x} label={`「${t().title}」の委託をやめる`} onClick={() => void cancel(t().task, t().title)} />
        </>
      }
    />
  );
}

/** Where the delegated tasks stand, said on the list of tasks, leading to their queue. */
export function DelegatedSummary() {
  const delegated = createAsync(() => delegatedQuery({ workgroup: workgroup() }));
  const queue = () => delegated()?.queue ?? [];
  const on = (stage: string) => queue().filter((t) => stageOf(t) === stage).length;
  return (
    <Show when={queue().length > 0}>
      <a class="delegated-summary" href="/tasks?tab=delegated">
        <Icon svg={icons.robot} size={16} />
        <span>委ねた仕事 {queue().length} 件</span>
        <For each={STAGES.filter(([s]) => on(s) > 0)}>
          {([s, name]) => (
            <Tag tone={stageLook(s).tone} icon={stageLook(s).icon}>
              {name} {on(s)}
            </Tag>
          )}
        </For>
        <span class="push" />
        <Icon svg={icons.chevronRight} size={16} />
      </a>
    </Show>
  );
}

/** The queue of delegated tasks, in the order they start, and the runs that ended. */
export function DelegatedPage(props: { tools: JSX.Element; detail: (id: string) => JSX.Element }) {
  const base = "/tasks?tab=delegated";
  const id = useParam("id");
  const delegated = createAsync(() => delegatedQuery({ workgroup: workgroup() }));
  // A place is one among all that wait, whatever is on screen.
  const line = createAsync(() => delegatedQuery());
  const tasks = createAsync(() => recordsQuery("tasks", "all"));
  const stages = createMemo(() =>
    STAGES.map(([stage, name]) => ({ stage, name, tasks: (delegated()?.queue ?? []).filter((t) => stageOf(t) === stage) })).filter(
      (s) => s.tasks.length > 0,
    ),
  );
  const order = createMemo(() => {
    const ids = stages().flatMap((s) => s.tasks.map((t) => t.task));
    return ids.map((i) => tasks()?.find((r) => r.id === i)).filter((r): r is Rec => r !== undefined);
  });
  return (
    <>
      <RecordsFrame name="delegated" base={base} title="委託" id={id()} order={order()} detail={props.detail}>
        <PageHeader
          title="委託"
          tags={<span class="sub">{(delegated()?.queue ?? []).length} 件</span>}
          actions={<span class="d-only">{props.tools}</span>}
          note={
            <>
              手代か外のエージェントに委ねたタスク。上から順に動く。順番を待つものには順番が付く。同時に動く数と一時停止は<a href="/settings/roles/worker">手代の設定</a>
              で決める。
            </>
          }
        />
        <Show when={delegated()} fallback={<Loading />}>
          {(d) => (
            <>
              <Show when={d().queue.length > 0} fallback={<EmptyState>並んでいる仕事はない</EmptyState>}>
                <For each={stages()}>
                  {(s) => (
                    <section class="stack" aria-label={s.name}>
                      <div class="row">
                        <h2>{s.name}</h2>
                        <span class="sub">{s.tasks.length}</span>
                      </div>
                      <List>
                        <For each={s.tasks}>
                          {(t) => (
                            <QueueRow
                              of={t}
                              href={withParams(base, { id: t.task })}
                              current={t.task === id()}
                              place={placeOf(line()?.queue, t.task)}
                              task={tasks()?.find((r) => r.id === t.task)}
                            />
                          )}
                        </For>
                      </List>
                    </section>
                  )}
                </For>
              </Show>
              <Show when={d().ended.length > 0}>
                <section class="stack" aria-label="済んだ仕事">
                  <div class="row">
                    <h2>済んだ仕事</h2>
                    <span class="sub">{d().ended.length}</span>
                  </div>
                  <List>
                    <For each={d().ended}>{(r) => <RunResult run={r} href={withParams(base, { id: r.task })} />}</For>
                  </List>
                </section>
              </Show>
            </>
          )}
        </Show>
      </RecordsFrame>
      <Show when={!id()}>
        <div class="m-float">
          <div class="m-toggles">{props.tools}</div>
        </div>
      </Show>
    </>
  );
}

/** One run: how it ended and what the worker left. */
export function RunResult(props: { run: WorkerRun; href?: string }) {
  const owner = useOwner();
  const r = () => props.run;
  return (
    <div class="item wraps worker-run" classList={{ picks: !!props.href }} role="listitem">
      <div class="item-main">
        <Show when={props.href} fallback={<span class="item-title">{r().title}</span>}>
          {(href) => (
            <a class="item-title item-pick" href={href()}>
              {r().title}
            </a>
          )}
        </Show>
        <div class="item-sub">
          <Tag tone={outcomeLook(r().outcome).tone} icon={outcomeLook(r().outcome).icon}>
            {outcomeText(r().outcome)}
          </Tag>
          <span class="mono">#{String(r().id)}</span>
          <span>{runnerText(r().runner)}</span>
          <Show when={r().attempt > 1}>
            <span>{r().attempt} 回目</span>
          </Show>
          <Show when={r().session}>
            <span>セッション {r().session}</span>
          </Show>
          <Show when={r().endedAt}>
            <span>{shortTime(r().endedAt, owner.tz())}</span>
          </Show>
          <a href={runHref(r())} aria-label={`「${r().title}」#${String(r().id)} の出力を見る`}>
            出力
          </a>
        </div>
      </div>
      <Show when={r().result}>
        <div class="item-more">
          <Disclosure title="残したもの">
            <pre class="rec-raw">{r().result}</pre>
          </Disclosure>
        </div>
      </Show>
    </div>
  );
}

/** A value, and what it was once it is no more: what is shown of it may be read while it is taken down. */
function createLast<T>(read: () => T | undefined) {
  return createMemo<T | undefined>((before) => read() ?? before);
}

/** How a delegated task stands, above what it says. */
export function Delegation(props: { task: Rec }) {
  const delegated = createAsync(() => delegatedQuery({ task: props.task.id }));
  const now = () => delegated()?.queue.find((t) => t.task === props.task.id);
  const seen = () => {
    const t = now();
    return t && isSeen(t) ? t.run : undefined;
  };
  const standing = createLast(now);
  const run = createLast(seen);
  const line = createAsync(() => delegatedQuery());
  const place = () => placeOf(line()?.queue, props.task.id);
  const ended = () => delegated()?.ended.find((r) => r.task === props.task.id);
  const last = createLast(ended);
  return (
    <Show when={valueOf(props.task, "Assignee") === WORKER}>
      <Section
        label="委託の様子"
        title="委託"
        level={3}
        action={
          <>
            <Show when={now()?.state === "still" || (isOver(props.task) && ended())}>
              <Button small onClick={() => void retry(props.task.id)}>
                並びに戻す
              </Button>
            </Show>
            <Show when={!!seen()}>
              <Watch run={run()!} title={props.task.title} />
            </Show>
            <Show when={!!now()}>
              <Button small onClick={() => void cancel(props.task.id, props.task.title)}>
                委託をやめる
              </Button>
            </Show>
          </>
        }
      >
        <Show when={!!now()}>
          <div class="row">
            <Standing of={standing()!} />
            <span class="sub">
              {runnerText(standing()!.runner)}
              <Show when={place() > 0}> · 並びの {place()} 番目</Show>
            </span>
            <Awaits of={standing()!} task={props.task} class="sub" />
            <Show when={standing()!.why}>
              <span class="sub">{standing()!.why}</span>
            </Show>
            <Show when={standing()!.run?.session}>
              <span class="sub">セッション {standing()!.run?.session}</span>
            </Show>
          </div>
        </Show>
        <Show when={!!seen()}>
          <RunAsk run={run()!} />
          <ProgressLine run={run()!} />
          <Show when={run()!.steps.length > 0}>
            <RunSteps run={run()!} />
          </Show>
        </Show>
        <Show when={!!ended()}>
          <List>
            <RunResult run={last()!} />
          </List>
        </Show>
      </Section>
    </Show>
  );
}
