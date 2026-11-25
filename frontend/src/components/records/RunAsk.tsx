import { createMemo, createSignal, For, Show } from "solid-js";
import { SessionAsk } from "~/components/ApprovalCard";
import { Button } from "~/components/ui";
import type { ApprovalAnswer } from "~/gen/famulen/v1/console_pb";
import type { RunQuestion } from "~/gen/famulen/v1/workers_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import type { WorkerRun } from "~/lib/workers";

function Ask(props: { run: bigint; question: RunQuestion; ticket: bigint }) {
  const [busy, setBusy] = createSignal(false);
  const once = async (run: () => Promise<unknown>, done: string) => {
    if (busy()) return;
    setBusy(true);
    await act(run, done);
    setBusy(false);
  };
  const about = () => ({ run: props.run, question: props.question.id });
  const answer = (given: ApprovalAnswer, replies: string[] = []) =>
    void once(() => api.workers.answerRun({ ...about(), answer: given, replies }), "答えを届けた");
  return (
    <div class="approval" role="group" aria-label="セッションの問い">
      <div class="row">
        <b class="asked-does grow">{props.question.approval?.deed}</b>
        <Show when={!props.question.truncated}>
          <Button
            small
            disabled={busy()}
            title="会話と Slack に、同じ問いを確認として出し直す"
            onClick={() => void once(() => api.workers.askRunAgain(about()), "もう一度尋ねた")}
          >
            再度聞く
          </Button>
        </Show>
      </div>
      <Show when={props.ticket > 0n}>
        <span class="sub">確認 #{props.ticket.toString()} でも尋ねている。どちらで答えても、答えは一度だけ届く。</span>
      </Show>
      <Show
        when={!props.question.truncated && props.question.approval}
        fallback={<p class="sub">ここでは読み切れない。Adroit で答える。</p>}
      >
        {(approval) => <SessionAsk approval={approval()} disabled={busy()} onAnswer={answer} />}
      </Show>
    </div>
  );
}

/** What a run's session waits on the owner for, to be answered here or asked again. */
export default function RunAsk(props: { run: WorkerRun }) {
  // A question is read again and again while it waits, and never changes:
  // what is being written in it stays.
  const kept = createMemo<RunQuestion[]>((before) => {
    const q = props.run.question;
    if (!q) return [];
    return before[0]?.id === q.id ? before : [q];
  }, []);
  return (
    <>
      <For each={kept()}>{(q) => <Ask run={props.run.id} question={q} ticket={props.run.question?.approval?.id ?? 0n} />}</For>
      <Show when={props.run.state === "waiting" && !props.run.question}>
        <p class="sub">何を尋ねているかを読めていない。急ぐなら Adroit で答える。</p>
      </Show>
    </>
  );
}
