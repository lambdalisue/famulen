import { createMemo, For, Show } from "solid-js";
import { Icon } from "~/components/ui";
import * as icons from "~/lib/icons";
import { outline, type WorkerRun } from "~/lib/workers";

const MARKS: { [status: string]: [string, string] } = {
  completed: [icons.circleCheck, "済"],
  in_progress: [icons.progress, "いま"],
  blocked: [icons.alertTriangle, "止まっている"],
};
const mark = (status: string) => MARKS[status] ?? [icons.circle, "これから"];

/** How far a run got in its plan, and what it does now. */
export function ProgressLine(props: { run: WorkerRun }) {
  return (
    <span class="run-progress">
      <Show when={props.run.steps.length > 0}>
        <span class="run-bar" role="progressbar" aria-label="進み具合" aria-valuenow={props.run.percent} aria-valuemin={0} aria-valuemax={100}>
          <span style={{ width: `${props.run.percent}%` }} />
        </span>
        <span class="mono">{props.run.percent}%</span>
      </Show>
      <Show when={props.run.doing}>
        <span class="run-doing">{props.run.doing}</span>
      </Show>
    </span>
  );
}

/** The plan a worker keeps, each step with how it stands. */
export function RunSteps(props: { run: WorkerRun }) {
  const steps = createMemo(() => outline(props.run.steps));
  return (
    <ol class="run-steps" aria-label="予定">
      <For each={steps()}>
        {({ step, depth }) => (
          <li class={`run-step ${step.status}`} style={{ "padding-left": `${depth * 18}px` }}>
            <Icon svg={mark(step.status)[0]} size={16} />
            <span class="sr">{mark(step.status)[1]}</span>
            <span class="run-step-text">
              {step.title}
              <Show when={step.note}>
                <span class="sub">{step.note}</span>
              </Show>
            </span>
          </li>
        )}
      </For>
    </ol>
  );
}
