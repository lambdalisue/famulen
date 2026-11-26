import { createAsync, useParams } from "@solidjs/router";
import { createEffect, createMemo, createSignal, on, onCleanup, onMount, Show } from "solid-js";
import { ProgressLine } from "~/components/records/RunProgress";
import RunBlock from "~/components/runs/RunBlocks";
import RunPlan from "~/components/runs/RunPlan";
import { Callout, EmptyState, Icon, Keyed, LinkButton, Loading, PageHeader, Segmented, Tag } from "~/components/ui";
import * as icons from "~/lib/icons";
import { type Block, blockKey, blocks, durationText, tokensText, usedSum } from "~/lib/runview";
import { watchRun } from "~/lib/runwatch";
import { usedText } from "~/lib/usage";
import { setView } from "~/lib/view";
import { isBadOutcome, runnerText, runOutputQuery, runText, type WorkerRun } from "~/lib/workers";

/** How near to its end the output counts as followed. */
const NEAR = 60;
/** How often the clock of a run that goes on moves. */
const TICK_MS = 1000;

function tone(r: WorkerRun) {
  if (r.state === "ended") return isBadOutcome(r.outcome) ? "danger" : "ok";
  if (r.state === "asking" || r.state === "waiting") return "warn";
  return r.state === "unreachable" ? "danger" : "aide";
}

/** The first block at or after an entry: what the run did then. */
const at = (all: Block[], seq: bigint) => all.find((b) => b.seq >= seq);

function RunOf(props: { id: string }) {
  const id = () => props.id;
  const first = createAsync(() => runOutputQuery(id()));
  const watched = watchRun(id, first);
  const output = () => watched.state();
  const run = () => output()?.run;
  const all = createMemo(() => blocks(watched.entries(), watched.parts()));
  const used = createMemo(() => usedSum(watched.entries()));
  const [now, setNow] = createSignal(Date.now());
  const [pane, setPane] = createSignal("stream");
  const [flashed, setFlashed] = createSignal("");
  let list: HTMLOListElement | undefined;
  let follows = true;

  createEffect(() => setView("タスク", "委託", run()?.title));
  onMount(() => {
    const timer = setInterval(() => setNow(Date.now()), TICK_MS);
    onCleanup(() => clearInterval(timer));
    // What is drawn later, such as Markdown, makes the output longer too.
    const grown = new ResizeObserver(() => {
      if (list && follows) list.scrollTop = list.scrollHeight;
    });
    createEffect(
      on(
        () => all().length,
        () => {
          if (!list) return;
          grown.disconnect();
          for (const line of list.children) grown.observe(line);
          if (follows) list.scrollTop = list.scrollHeight;
        },
      ),
    );
    onCleanup(() => grown.disconnect());
  });

  const jump = (seq: bigint) => {
    const b = at(all(), seq);
    if (!b) return;
    setPane("stream");
    follows = false;
    const key = blockKey(b);
    setFlashed(key);
    requestAnimationFrame(() => document.getElementById(`run-block-${key}`)?.scrollIntoView({ block: "center", behavior: "smooth" }));
  };

  const lasted = (r: WorkerRun) => {
    if (!r.startedAt) return "";
    return durationText((r.endedAt ? Number(r.endedAt) * 1000 : now()) - Number(r.startedAt) * 1000);
  };

  return (
    <Show when={run()} fallback={<Loading />}>
      {(r) => (
        <div class="run-page" data-pane={pane()}>
          <PageHeader
            title={r().title}
            back={{ href: "/tasks?tab=delegated", label: "委託" }}
            tags={<Tag tone={tone(r())}>{runText(r())}</Tag>}
            actions={
              <LinkButton small href={`/tasks?tab=delegated&id=${r().task}`}>
                タスクを見る
              </LinkButton>
            }
            note={
              <span class="run-about">
                <span class="mono">#{String(r().id)}</span>
                <span>{runnerText(r().runner)}</span>
                <Show when={lasted(r())}>
                  {(text) => (
                    <span class="run-lasted">
                      <Icon svg={icons.clock} size={13} /> {text()}
                    </span>
                  )}
                </Show>
                <Show when={r().session}>
                  <span>セッション {r().session}</span>
                </Show>
                <Show when={used().input + used().output > 0n} fallback={<Show when={(output()?.usage.length ?? 0) > 0}><span>{usedText(output()?.usage ?? [])}</span></Show>}>
                  <span class="run-used" title={used().model}>
                    <Icon svg={icons.coin} size={13} /> 入力 {tokensText(used().input)}（うちキャッシュ {tokensText(used().cached)}）・出力 {tokensText(used().output)}
                  </span>
                </Show>
                <Show when={watched.live() && r().state !== "ended"}>
                  <span class="run-live-dot">流れている</span>
                </Show>
              </span>
            }
          />
          <Show when={output()?.why}>
            {(why) => (
              <Callout tone="warn">
                いまの様子を読めなかった
                <div class="sub">{why()}</div>
              </Callout>
            )}
          </Show>
          <div class="run-progress-row">
            <ProgressLine run={r()} />
          </div>
          <Segmented
            class="run-panes"
            label="見るもの"
            items={[
              ["stream", "出力", icons.message],
              ["plan", "段取り", icons.listCheck],
            ]}
            value={pane()}
            onChoose={setPane}
          />
          <div class="run-body">
            <section class="run-plan" aria-label="進み具合">
              <RunPlan run={r()} changes={output()?.changes ?? []} now={now()} onJump={jump} />
            </section>
            <section class="run-output" aria-label="出力">
              <ol
                class="run-lines"
                ref={list}
                tabindex="0"
                aria-label="働き手の出力"
                aria-live="off"
                onScroll={(e) => {
                  const t = e.currentTarget;
                  follows = t.scrollHeight - t.scrollTop - t.clientHeight < NEAR;
                }}
              >
                <Show when={all().length === 0}>
                  <li>
                    <EmptyState>出力はまだない</EmptyState>
                  </li>
                </Show>
                <Keyed each={all()} by={blockKey}>
                  {(b) => (
                    <li id={`run-block-${blockKey(b())}`} class={`run-line ${b().kind}`} classList={{ flash: flashed() === blockKey(b()) }}>
                      <RunBlock b={b()} run={r()} now={now()} />
                    </li>
                  )}
                </Keyed>
                <Show when={r().state === "waiting" && !all().some((b) => b.kind === "question" && b.id === r().question?.id)}>
                  <li class="run-line question">
                    <RunBlock b={{ kind: "question", seq: 0n, at: 0n, id: r().question?.id ?? "", tool: "", what: r().question?.approval?.deed ?? "", withdrawn: false }} run={r()} now={now()} />
                  </li>
                </Show>
              </ol>
            </section>
          </div>
        </div>
      )}
    </Show>
  );
}

/** One run of a delegated task as it goes on: what its worker writes and does as it does it, and its plan. Another run is followed anew. */
export default function Run() {
  const params = useParams();
  return (
    <Show when={params.id} keyed>
      {(id) => <RunOf id={id} />}
    </Show>
  );
}
