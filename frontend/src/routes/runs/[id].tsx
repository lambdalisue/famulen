import { createAsync, revalidate, useParams } from "@solidjs/router";
import { createEffect, createSignal, on, onCleanup, onMount, Show } from "solid-js";
import Markdown from "~/components/Markdown";
import { ProgressLine, RunSteps } from "~/components/records/RunProgress";
import { Callout, Disclosure, EmptyState, Keyed, LinkButton, Loading, PageHeader, Tag } from "~/components/ui";
import type { WorkerLine } from "~/gen/famulen/v1/workers_pb";
import { shortTime } from "~/lib/dates";
import { useOwner } from "~/lib/owner";
import { setView } from "~/lib/view";
import { isBadOutcome, lastedText, runnerText, runOutputQuery, runText, type WorkerRun } from "~/lib/workers";

/** How often what goes on is read anew, and what has ended. */
const READ_MS = 3000;
const READ_ENDED_MS = 30_000;

/** How near to its end the output counts as followed. */
const NEAR = 60;

const STARTS: { [kind: string]: string } = { asked: "頼み", answered: "道具の答え" };

function Line(props: { line: WorkerLine }) {
  const owner = useOwner();
  const l = () => props.line;
  return (
    <li class={`run-line ${l().kind}`}>
      <Show when={STARTS[l().kind]} fallback={l().kind === "called" ? <code>{l().text}</code> : <Markdown text={l().text} />}>
        {(title) => (
          <Disclosure title={title()}>
            <pre class="rec-raw">{l().text}</pre>
          </Disclosure>
        )}
      </Show>
      <span class="run-line-at">{shortTime(l().at / 1000n, owner.tz())}</span>
    </li>
  );
}

function tone(r: WorkerRun) {
  if (r.state === "ended") return isBadOutcome(r.outcome) ? "danger" : "ok";
  if (r.state === "asking" || r.state === "waiting") return "warn";
  return r.state === "unreachable" ? "danger" : "aide";
}

/** One run of a delegated task as it goes on: its plan, and what its worker writes. */
export default function Run() {
  const params = useParams();
  const id = () => params.id ?? "0";
  const output = createAsync(() => runOutputQuery(id()));
  const run = () => output()?.run;
  const [now, setNow] = createSignal(Date.now());
  let list: HTMLOListElement | undefined;
  let follows = true;

  createEffect(() => setView("タスク", "委託", run()?.title));
  onMount(() => {
    let timer: ReturnType<typeof setTimeout>;
    let left = false;
    const again = async () => {
      try {
        if (!document.hidden) await revalidate(runOutputQuery.keyFor(id()));
      } finally {
        setNow(Date.now());
        if (!left) timer = setTimeout(() => void again(), run()?.state === "ended" ? READ_ENDED_MS : READ_MS);
      }
    };
    timer = setTimeout(() => void again(), READ_MS);
    onCleanup(() => {
      left = true;
      clearTimeout(timer);
    });
    // What is drawn later, such as Markdown, makes the output longer too.
    const grown = new ResizeObserver(() => {
      if (list && follows) list.scrollTop = list.scrollHeight;
    });
    createEffect(
      on(
        () => output()?.lines.length,
        () => {
          if (!list) return;
          grown.disconnect();
          for (const line of list.children) grown.observe(line);
        },
      ),
    );
    onCleanup(() => grown.disconnect());
  });

  return (
    <Show when={run()} fallback={<Loading />}>
      {(r) => (
        <div class="run-page">
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
                <Show when={lastedText(r(), now())}>{(text) => <span>{text()}</span>}</Show>
                <Show when={r().session}>
                  <span>セッション {r().session}</span>
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
          <div class="run-body">
            <section class="run-plan" aria-label="進み具合">
              <ProgressLine run={r()} />
              <Show when={r().steps.length > 0} fallback={<EmptyState>予定はまだ書かれていない</EmptyState>}>
                <RunSteps run={r()} />
              </Show>
            </section>
            <section class="run-output" aria-label="出力">
              <ol
                class="run-lines"
                ref={list}
                tabindex="0"
                aria-label="働き手の出力"
                onScroll={(e) => {
                  const t = e.currentTarget;
                  follows = t.scrollHeight - t.scrollTop - t.clientHeight < NEAR;
                }}
              >
                <Show when={(output()?.lines.length ?? 0) === 0}>
                  <li>
                    <EmptyState>出力はまだない</EmptyState>
                  </li>
                </Show>
                <Keyed each={output()?.lines ?? []} by={(l) => l.id}>
                  {(l) => <Line line={l()} />}
                </Keyed>
              </ol>
            </section>
          </div>
        </div>
      )}
    </Show>
  );
}
