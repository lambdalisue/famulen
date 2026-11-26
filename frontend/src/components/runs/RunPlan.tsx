import { createMemo, createSignal, For, Show } from "solid-js";
import { EmptyState, Icon, Segmented } from "~/components/ui";
import type { StepChange } from "~/gen/famulen/v1/workers_pb";
import { clock } from "~/lib/dates";
import * as icons from "~/lib/icons";
import { useOwner } from "~/lib/owner";
import { durationText, PLAN_FROM, type Span, spans } from "~/lib/runview";
import { outline, type WorkerRun } from "~/lib/workers";

const MARKS: { [status: string]: [string, string] } = {
  completed: [icons.circleCheck, "済"],
  in_progress: [icons.progress, "いま"],
  blocked: [icons.alertTriangle, "止まっている"],
};
const mark = (status: string) => MARKS[status] ?? [icons.circle, "これから"];

const CHANGES: { [change: string]: string } = { added: "足した", title: "名を変えた", note: "備考を書いた", removed: "外した" };
const STATUSES: { [status: string]: string } = { pending: "これから", in_progress: "始めた", completed: "済んだ", blocked: "止まった" };

const changeText = (c: StepChange) => (c.change === "status" ? (STATUSES[c.to] ?? c.to) : (CHANGES[c.change] ?? c.change));

type Props = {
  run: WorkerRun;
  changes: StepChange[];
  now: number;
  /** Go to what the run did at this entry. */
  onJump: (seq: bigint) => void;
};

function lasted(s: Span, now: number): string {
  if (!s.from) return "";
  return durationText((s.to ? Number(s.to) : now) - Number(s.from));
}

function Tree(props: Props & { rows: Span[] }) {
  const owner = useOwner();
  const [open, setOpen] = createSignal("");
  return (
    <ol class="run-steps" aria-label="予定">
      <For each={props.rows}>
        {(s) => (
          <li class={`run-step ${s.step.status}`} style={{ "padding-left": `${s.depth * 18}px` }}>
            <Icon svg={mark(s.step.status)[0]} size={16} />
            <span class="sr">{mark(s.step.status)[1]}</span>
            <div class="run-step-text">
              <button
                type="button"
                class="run-step-title"
                aria-expanded={open() === s.step.id}
                onClick={() => setOpen((o) => (o === s.step.id ? "" : s.step.id))}
              >
                {s.step.title}
                <Show when={s.step.weight !== 1}>
                  <span class="run-weight" title="兄弟の間での重み">
                    ×{s.step.weight}
                  </span>
                </Show>
                <Show when={lasted(s, props.now)}>{(t) => <span class="run-took">{t()}</span>}</Show>
              </button>
              <Show when={s.step.note}>
                <span class="sub">{s.step.note}</span>
              </Show>
              <Show when={open() === s.step.id}>
                <div class="run-step-detail">
                  <Show when={s.step.description}>
                    <p>{s.step.description}</p>
                  </Show>
                  <ol class="run-step-changes" aria-label="変わったこと">
                    <For each={s.changes}>
                      {(c) => (
                        <li>
                          <button type="button" class="link" onClick={() => props.onJump(c.seq)}>
                            {clock(c.at / 1000n, owner.tz())} {changeText(c)}
                          </button>
                        </li>
                      )}
                    </For>
                  </ol>
                </div>
              </Show>
            </div>
            <Show when={s.seq}>
              {(seq) => (
                <button type="button" class="run-jump" title="この段の時の出力へ" aria-label={`「${s.step.title}」の時の出力へ`} onClick={() => props.onJump(seq())}>
                  <Icon svg={icons.chevronRight} size={14} />
                </button>
              )}
            </Show>
          </li>
        )}
      </For>
    </ol>
  );
}

function Time(props: Props & { rows: Span[] }) {
  const owner = useOwner();
  const from = () => Number(props.run.startedAt || props.run.createdAt) * 1000;
  const to = () => (props.run.endedAt ? Number(props.run.endedAt) * 1000 : props.now);
  const width = () => Math.max(1, to() - from());
  const place = (at: number) => `${Math.min(100, Math.max(0, ((at - from()) / width()) * 100))}%`;
  // From its start, which a run of minutes would not tell apart by the clock.
  const ticks = createMemo(() => [0.5, 1].map((f) => f * width()));
  return (
    <div class="run-time" role="list" aria-label="時間の流れ">
      <div class="run-time-axis" aria-hidden="true">
        <span style={{ left: "0%" }}>{clock(Math.floor(from() / 1000), owner.tz())}</span>
        <For each={ticks()}>{(t) => <span style={{ left: place(from() + t) }}>+{durationText(t)}</span>}</For>
      </div>
      <For each={props.rows}>
        {(s) => (
          <div class={`run-time-row ${s.step.status}`} role="listitem">
            <span class="run-time-name" style={{ "padding-left": `${s.depth * 12}px` }}>
              {s.step.title}
            </span>
            <span class="run-time-track">
              <Show when={s.from}>
                {(start) => (
                  <button
                    type="button"
                    class="run-time-bar"
                    style={{ left: place(Number(start())), width: `max(4px, calc(${place(s.to ? Number(s.to) : to())} - ${place(Number(start()))}))` }}
                    title={`${s.step.title}：${lasted(s, props.now)}`}
                    aria-label={`「${s.step.title}」 ${lasted(s, props.now)}、この時の出力へ`}
                    onClick={() => s.seq !== undefined && props.onJump(s.seq)}
                  />
                )}
              </Show>
            </span>
          </div>
        )}
      </For>
    </div>
  );
}

/** A run's plan as a tree or along the time it took, each step leading to what the run did then. */
export default function RunPlan(props: Props) {
  const [view, setView] = createSignal("tree");
  const rows = createMemo(() => spans(outline(props.run.steps), props.changes));
  return (
    <div class="run-plan-view">
      <div class="row">
        <Segmented
          label="予定の見方"
          brief
          items={[
            ["tree", "木", icons.sitemap],
            ["time", "時間の流れ", icons.timeline],
          ]}
          value={view()}
          onChoose={setView}
        />
        <Show when={PLAN_FROM[props.run.planFrom]}>
          {(from) => (
            <span class={`run-from ${props.run.planFrom}`}>
              <Icon svg={props.run.planFrom === "notes" ? icons.robot : icons.listCheck} size={13} />
              {from()}
            </span>
          )}
        </Show>
      </div>
      <Show when={props.run.steps.length > 0} fallback={<EmptyState>段取りはまだ書かれていない。割合は出さず、最後に書いたことと道具を見せる</EmptyState>}>
        <Show when={view() === "time"} fallback={<Tree {...props} rows={rows()} />}>
          <Time {...props} rows={rows()} />
        </Show>
      </Show>
    </div>
  );
}
