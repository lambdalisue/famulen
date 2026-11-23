import { shortTime } from "~/lib/dates";
import { A, createAsync } from "@solidjs/router";
import { For, Show } from "solid-js";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import * as icons from "~/lib/icons";
import { isOver, type Rec, valueOf, WORKER } from "~/lib/records";
import { useOwner } from "~/lib/owner";
import { workgroup } from "~/lib/workgroup";
import { type DelegatedTask, delegatedQuery, isBad, outcomeText, runnerText, standingText, type WorkerRun } from "~/lib/workers";
import { Button, Icon, IconButton, Tag } from "~/components/ui";

const retry = (id: string) => act(() => api.workers.retryDelegated({ id }), "並びに戻した");
const cancel = (id: string) => act(() => api.workers.cancelDelegated({ id }), "やめた");

function Standing(props: { of: DelegatedTask }) {
  return (
    <span class={`tag${isBad(props.of) ? " bad" : props.of.state === "asking" || props.of.state === "waiting" ? " warn" : ""}`}>
      {standingText(props.of)}
    </span>
  );
}

/** Where every delegated task stands, in the order they start. */
export function DelegatedQueue() {
  const delegated = createAsync(() => delegatedQuery({ workgroup: workgroup() }));
  return (
    <Show when={(delegated()?.queue ?? []).length > 0}>
      <section class="rec-group" aria-label="委ねた仕事">
        <h2>委ねた仕事</h2>
        <div class="rec-rows">
          <For each={delegated()?.queue}>
            {(t) => (
              <div class="rec-row" style={{ cursor: "default" }} data-delegated={t.task}>
                <Icon svg={icons.robot} size={16} />
                <span class="rec-main">
                  <A href={`/tasks?id=${t.task}`}>
                    <b>{t.title}</b>
                  </A>
                  <span class="rec-sub">
                    <Standing of={t} />
                    <span>{runnerText(t.runner)}</span>
                    <Show when={(t.run?.attempt ?? 0) > 1}>
                      <span>{t.run?.attempt} 回目</span>
                    </Show>
                    <Show when={t.why}>
                      <span>{t.why}</span>
                    </Show>
                  </span>
                </span>
                <Show when={t.state === "still"}>
                  <Button small onClick={() => void retry(t.task)}>
                    並びに戻す
                  </Button>
                </Show>
                <IconButton plain icon={icons.x} label={`「${t.title}」の委託をやめる`} onClick={() => void cancel(t.task)} />
              </div>
            )}
          </For>
        </div>
      </section>
    </Show>
  );
}

/** One run: how it ended and what the worker left. */
export function RunResult(props: { run: WorkerRun }) {
  const owner = useOwner();
  const r = () => props.run;
  return (
    <div role="listitem" class="item worker-run">
      <div class="grow">
        <div>
          <span class="sub mono">#{String(r().id)}</span> {r().title}{" "}
          <Tag tone={["failed", "refused"].includes(r().outcome) ? "danger" : undefined}>{outcomeText(r().outcome)}</Tag>
        </div>
        <div class="sub">
          {runnerText(r().runner)}
          <Show when={r().attempt > 1}> · {r().attempt} 回目</Show>
          <Show when={r().session}> · セッション {r().session}</Show>
          <Show when={r().endedAt}> · {shortTime(r().endedAt, owner.tz())}</Show>
        </div>
        <Show when={r().result}>
          <details>
            <summary class="sub">残したもの</summary>
            <pre class="rec-raw" style={{ "white-space": "pre-wrap" }}>
              {r().result}
            </pre>
          </details>
        </Show>
      </div>
    </div>
  );
}

/** How a delegated task stands, above what it says. */
export function Delegation(props: { task: Rec }) {
  const delegated = createAsync(() => delegatedQuery({ task: props.task.id }));
  const now = () => delegated()?.queue.find((t) => t.task === props.task.id);
  const last = () => delegated()?.ended.find((r) => r.task === props.task.id);
  return (
    <Show when={valueOf(props.task, "Assignee") === WORKER}>
      <section class="rec-section" aria-label="委託の様子">
        <div class="row">
          <Show when={now()}>
            {(t) => (
              <>
                <Standing of={t()} />
                <span class="sub grow">
                  {t().why}
                  <Show when={t().run?.session}> セッション {t().run?.session}</Show>
                </span>
              </>
            )}
          </Show>
          <Show when={now()?.state === "still" || (isOver(props.task) && last())}>
            <Button small onClick={() => void retry(props.task.id)}>
              並びに戻す
            </Button>
          </Show>
          <Show when={now()}>
            <Button small onClick={() => void cancel(props.task.id)}>
              委託をやめる
            </Button>
          </Show>
        </div>
        <Show when={last()}>{(r) => <RunResult run={r()} />}</Show>
      </section>
    </Show>
  );
}
