import { createSignal, Show } from "solid-js";
import { Button, Callout, TextArea, whenConfirmed } from "~/components/ui";
import type { MemoryItem } from "~/gen/famulen/v1/memory_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import * as icons from "~/lib/icons";
import { useOwner } from "~/lib/owner";

/** What is being written on each judgment, kept while the page is read again or left. */
const [drafts, setDrafts] = createSignal<{ [id: string]: string | undefined }>({});

/** What the owner decides of a pending judgment, where it is read. */
export default function PendingActions(props: { item: MemoryItem; onGone?: () => void }) {
  const owner = useOwner();
  const id = () => props.item.id;
  const draft = () => drafts()[id()];
  const write = (text: string | undefined) => setDrafts((d) => ({ ...d, [id()]: text }));
  const [busy, setBusy] = createSignal(false);
  const run = async <T,>(change: () => Promise<T>, done: string) => {
    setBusy(true);
    const answered = await act(change, done);
    setBusy(false);
    return answered !== undefined;
  };
  const answer = async (adopt: boolean) => {
    const sent = draft() ?? "";
    const done = await run(() => api.memory.answerPending({ id: id(), adopt, words: sent }), adopt ? "採用した" : "判断を送った");
    if (done && (draft() ?? "") === sent) write(undefined);
  };
  const reject = () =>
    whenConfirmed(
      "この判断を却下する",
      "却下する",
      async () => {
        const done = await run(() => api.memory.rejectPending({ id: id(), reason: draft() ?? "" }), "却下した");
        if (!done) return;
        write(undefined);
        props.onGone?.();
      },
      "忘却に移る。忘却からいつでも保留に戻せる。",
    );
  return (
    <div class="pending-actions">
      <Show when={props.item.answer}>
        {(a) => (
          <Callout
            icon={icons.check}
            action={
              <Button
                small
                icon={icons.arrowBackUp}
                disabled={busy()}
                onClick={() => void run(() => api.memory.withdrawAnswer({ id: id() }), "保留に戻した")}
              >
                保留に戻す
              </Button>
            }
          >
            <b>{a().adopted ? "採用した" : "判断を書いた"}</b>
            <Show when={a().words}> ：{a().words}</Show>
            <div class="sub">
              {a().date} {a().time} · 次の夢が記憶に入れる。{owner.aide()} は次の会話から従う
            </div>
          </Callout>
        )}
      </Show>
      <Show when={draft() !== undefined}>
        <TextArea
          class="pending-words"
          aria-label={`${props.item.text} への判断`}
          placeholder="どう判断するか。採用や却下に添えてもよい"
          value={draft()}
          onInput={(e) => write(e.currentTarget.value)}
        />
      </Show>
      <div class="row">
        <Show
          when={draft() !== undefined}
          fallback={
            <Show when={!props.item.answer}>
              <Button small icon={icons.pencil} onClick={() => write("")}>
                判断を書く
              </Button>
            </Show>
          }
        >
          <Button small variant="primary" disabled={busy() || !draft()?.trim()} onClick={() => void answer(false)}>
            判断を送る
          </Button>
        </Show>
        <Show when={!props.item.answer}>
          <Button small variant="go" icon={icons.check} disabled={busy()} onClick={() => void answer(true)}>
            採用
          </Button>
        </Show>
        <Button small variant="danger" icon={icons.x} disabled={busy()} onClick={() => void reject()}>
          却下
        </Button>
        <Show when={draft() !== undefined}>
          <Button small variant="ghost" onClick={() => write(undefined)}>
            書くのをやめる
          </Button>
        </Show>
      </div>
    </div>
  );
}
