import { createSignal, Show } from "solid-js";
import { Button, Form, TextField } from "~/components/ui";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import type { WorkerRun } from "~/lib/workers";

/** Whether the agent is at work on a turn something more can join. */
const hears = (r: WorkerRun) => r.runner === "direct" && ["running", "idle", "waiting"].includes(r.state) && ["", "implementing"].includes(r.stage);

const STAGES: { [stage: string]: string } = {
  implementing: "実装中",
  checking: "検査中",
  reviewing: "レビュー中",
  publishing: "push と PR",
  ci: "CI 待ち",
  merging: "マージ",
  deploying: "反映中",
  tidying: "片づけ中",
  done: "仕上げた",
  stopped: "仕上げを止めた",
};

/** Where Famulen is in finishing the work of a run, and what failed. */
export function RunStage(props: { run: WorkerRun }) {
  return (
    <Show when={props.run.stage}>
      <div class="run-stage" role="group" aria-label="仕上げ">
        <div class="row">
          <span class="sub">仕上げ: {STAGES[props.run.stage] ?? props.run.stage}</span>
          <Show when={props.run.pullRequest}>
            <a href={props.run.pullRequest} target="_blank" rel="noreferrer">
              PR を開く
            </a>
          </Show>
        </div>
        <Show when={props.run.stageNote}>
          <pre class="rec-raw">{props.run.stageNote}</pre>
        </Show>
      </div>
    </Show>
  );
}

/** Something more said to a run at work, which joins the turn it is in. */
export default function RunMore(props: { run: WorkerRun }) {
  const [text, setText] = createSignal("");
  const send = async () => {
    const said = text().trim();
    if (!said) return;
    const sent = await act(() => api.workers.tellRun({ run: props.run.id, text: said }), "指示を送った");
    if (sent) setText("");
  };
  return (
    <Show when={hears(props.run)}>
      <Form class="row run-more" onSubmit={send}>
        <TextField class="grow" aria-label="エージェントへの追加の指示" placeholder="追加の指示" value={text()} onInput={(e) => setText(e.currentTarget.value)} />
        <Button small type="submit" disabled={!text().trim()}>
          送る
        </Button>
      </Form>
    </Show>
  );
}
