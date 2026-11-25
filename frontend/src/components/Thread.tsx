import { createEffect, For, on, onCleanup, onMount, Show } from "solid-js";
import Message from "~/components/Message";
import { ErrorState, IconButton } from "~/components/ui";
import { type ConsoleMessage, Speaker } from "~/gen/famulen/v1/console_pb";
import * as icons from "~/lib/icons";
import { pointOut, offsetIn } from "~/lib/scroll";
import type { createThread } from "~/lib/thread";

/** A message and the replies to it, over or under the conversation. Its newest stays in sight until the owner scrolls up. */
export default function Thread(props: {
  thread: ReturnType<typeof createThread>;
  root: bigint;
  aide: string;
  tz: string;
  onReply: (m: ConsoleMessage) => void;
  onClose: () => void;
}) {
  let list: HTMLDivElement | undefined;
  let said: HTMLDivElement | undefined;
  let follows = true;
  onMount(() => {
    if (!list || !said) return;
    const seen = list;
    const sized = new ResizeObserver(() => {
      if (follows) seen.scrollTop = seen.scrollHeight;
    });
    sized.observe(seen);
    sized.observe(said);
    onCleanup(() => sized.disconnect());
  });
  // What the owner just said is seen, wherever they read.
  createEffect(
    on(
      () => props.thread.messages().at(-1),
      (last, before) => {
        if (!list || !last || last.id === before?.id || last.who !== Speaker.OWNER) return;
        follows = true;
        list.scrollTop = list.scrollHeight;
      },
      { defer: true },
    ),
  );
  const go = (id: bigint) => {
    const to = list?.querySelector<HTMLElement>(`[data-id="${id}"]`);
    if (!list || !to) return;
    follows = false;
    list.scrollTop += offsetIn(list, to);
    pointOut(to);
  };
  let was = { top: 0, height: 0, room: 0 };
  return (
    <section class="thread" aria-label="スレッド">
      <div class="thread-head">
        <b class="grow">スレッド</b>
        <IconButton icon={icons.x} label="スレッドを閉じる" variant="ghost" onClick={() => props.onClose()} />
      </div>
      <div
        class="thread-said"
        ref={list}
        role="log"
        aria-label="スレッドの発言"
        tabindex="0"
        onScroll={(e) => {
          const { scrollTop: top, scrollHeight: height, clientHeight: room } = e.currentTarget;
          if (height - top - room < 40) follows = true;
          else if (top < was.top && height === was.height && room === was.room) follows = false;
          was = { top, height, room };
        }}
      >
        <div class="said" ref={said}>
          <Show when={props.thread.failed()}>{(error) => <ErrorState error={error()} onRetry={() => void props.thread.retry()} />}</Show>
          <For each={props.thread.messages()}>
            {(m) => (
              <Message
                m={m}
                aide={props.aide}
                tz={props.tz}
                quote={m.replyTo?.id === props.root ? undefined : m.replyTo}
                onReply={props.onReply}
                onQuote={go}
              />
            )}
          </For>
        </div>
      </div>
    </section>
  );
}
