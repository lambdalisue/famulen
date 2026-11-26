import { createMemo, For, Show } from "solid-js";
import { SessionAsk } from "~/components/ApprovalCard";
import { Button, SendTogether } from "~/components/ui";
import { ApprovalAnswer } from "~/gen/famulen/v1/console_pb";
import type { RunQuestion } from "~/gen/famulen/v1/workers_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import type { WorkerRun } from "~/lib/workers";

function Ask(props: { run: bigint; question: RunQuestion; ticket: bigint; adroit: boolean }) {
  const about = () => ({ run: props.run, question: props.question.id });
  const answer = (given: ApprovalAnswer, replies: string[] = []) =>
    act(() => api.workers.answerRun({ ...about(), answer: given, replies }), "答えを届けた");
  return (
    <SendTogether>
      <div class="approval asked-in" role="group" aria-label="セッションの問い">
        <div class="row">
          <b class="asked-does grow">{props.question.approval?.deed}</b>
          <Show when={props.adroit && !props.question.truncated}>
            <Button
              small
              title="会話と Slack に、同じ問いを確認として出し直す"
              onClick={() => act(() => api.workers.askRunAgain(about()), "もう一度尋ねた")}
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
          fallback={<p class="sub">{props.adroit ? "ここでは読み切れない。Adroit で答える。" : props.ticket > 0n ? "ここでは読み切れない。受け箱の確認で答える。" : "ここでは読み切れない。"}</p>}
        >
          {(approval) => <SessionAsk approval={approval()} onAnswer={answer} />}
        </Show>
        <Show when={!props.adroit && props.ticket === 0n}>
          <div class="row">
            <Button small title="答えずに、エージェントに自分で決めさせる" onClick={() => answer(ApprovalAnswer.DENY)}>
              答えずに任せる
            </Button>
          </div>
        </Show>
      </div>
    </SendTogether>
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
      <For each={kept()}>{(q) => <Ask run={props.run.id} question={q} ticket={props.run.question?.approval?.id ?? 0n} adroit={props.run.runner === "adroit"} />}</For>
      <Show when={props.run.state === "waiting" && !props.run.question}>
        <p class="sub">{props.run.runner === "adroit" ? "何を尋ねているかを読めていない。急ぐなら Adroit で答える。" : "何を尋ねているかを読めていない。受け箱の確認で答える。"}</p>
      </Show>
    </>
  );
}
