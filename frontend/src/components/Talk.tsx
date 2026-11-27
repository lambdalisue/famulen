import { createEffect, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";
import Message from "~/components/Message";
import { Button, ErrorState, Icon, Loading } from "~/components/ui";
import { type ConsoleMessage, type Conversation, Speaker } from "~/gen/famulen/v1/console_pb";
import { dateTimeText, shortTime } from "~/lib/dates";
import * as icons from "~/lib/icons";
import { shortTokens } from "~/lib/models";
import { offsetIn, pointOut } from "~/lib/scroll";
import { type createTalk, go, NOTICES, SOURCES, titleOf } from "~/lib/talks";

/** What stands over what is said: which conversation it is, the way back to the others, and the way to another. */
export function TalkHead(props: { talk: Conversation | undefined; fresh: boolean; elsewhere: number; tz: string }) {
  const c = () => props.talk;
  return (
    <header class="talk-head">
      <Back elsewhere={props.elsewhere} />
      <Show
        when={!props.fresh}
        fallback={
          <div class="talk-name">
            <b>新しい話</b>
            <span class="sub">これまでの話は引き継がない</span>
          </div>
        }
      >
        <div class="talk-name">
          <b title={titleOf(c())}>{titleOf(c())}</b>
          <Show when={c()}>
            {(talk) => (
              <details class="talk-about">
                <summary>
                  <Show when={SOURCES[talk().source]}>{(from) => <span>{from()}</span>}</Show>
                  <span>{shortTime(talk().usedAt, props.tz)}</span>
                  <Icon svg={icons.chevronDown} size={12} />
                </summary>
                <dl class="talk-facts">
                  <dt>始まり</dt>
                  <dd>{dateTimeText(talk().openedAt, props.tz)}</dd>
                  <dt>やりとり</dt>
                  <dd>{Number(talk().spent?.turns ?? 0n)} 回</dd>
                  <dt>いまの文脈</dt>
                  <dd>{shortTokens(Number(talk().contextTokens))} トークン。増えると自動で縮む。発言は消えない。</dd>
                </dl>
              </details>
            )}
          </Show>
        </div>
        <Button small icon={icons.messagePlus} onClick={() => go({ kind: "new" })}>
          新しい話
        </Button>
      </Show>
    </header>
  );
}

/** The way back to the list, and whether something waits there. */
function Back(props: { elsewhere: number }) {
  const back = () => `話の一覧へ${props.elsewhere > 0 ? `（ほかに ${props.elsewhere} 件が待っている）` : ""}`;
  return (
    <button type="button" class="btn ghost small talk-back" aria-label={back()} title={back()} onClick={() => go({ kind: "list" })}>
      <Icon svg={icons.chevronLeft} size={18} />
      <Show when={props.elsewhere > 0}>
        <span class="talk-count warn">{props.elsewhere}</span>
      </Show>
    </button>
  );
}

/** What stands over what was told unasked. */
export function NoticesHead(props: { elsewhere: number }) {
  return (
    <header class="talk-head">
      <Back elsewhere={props.elsewhere} />
      <div class="talk-name">
        <b>通知</b>
        <span class="sub">返信すると、その知らせの話になる</span>
      </div>
    </header>
  );
}

/** What is said in one conversation. Its newest stays in sight until the owner scrolls up. */
export default function Talk(props: {
  talk: ReturnType<typeof createTalk>;
  id: bigint;
  aide: string;
  tz: string;
  working: string;
  onReply: (m: ConsoleMessage) => void;
}) {
  let list: HTMLDivElement | undefined;
  let said: HTMLDivElement | undefined;
  let follows = true;
  const [marked, setMarked] = createSignal<bigint>();
  // Where what is new begins stays marked for as long as it is open.
  createEffect(
    on(
      () => props.talk.there(),
      (there) => {
        const to = props.talk.readTo();
        if (!there || props.talk.failed() || to === undefined || marked() !== undefined) return;
        setMarked(props.talk.messages().find((m) => m.who === Speaker.AIDE && m.id > to)?.id ?? 0n);
      },
    ),
  );
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
  createEffect(
    on(
      () => props.talk.messages().at(-1),
      (last, before) => {
        if (!list || !last || last.id === before?.id || last.who !== Speaker.OWNER) return;
        follows = true;
        list.scrollTop = list.scrollHeight;
      },
      { defer: true },
    ),
  );
  const goTo = (id: bigint) => {
    const to = list?.querySelector<HTMLElement>(`[data-id="${id}"]`);
    if (!list || !to) return;
    follows = false;
    list.scrollTop += offsetIn(list, to);
    pointOut(to);
  };
  // What was being read stays where it is while older notices are put above it.
  const readBack = async () => {
    const id = props.talk.messages()[0]?.id;
    const placed = () => {
      const first = list?.querySelector<HTMLElement>(`[data-id="${id}"]`);
      return list && first ? list.scrollTop + offsetIn(list, first) : undefined;
    };
    const from = placed();
    follows = false;
    await props.talk.older();
    const to = placed();
    if (list && from !== undefined && to !== undefined) list.scrollTop += to - from;
  };
  let was = { top: 0, height: 0, room: 0 };
  return (
    <div
      class="messages talk-said"
      ref={list}
      role="log"
      aria-label={props.id === NOTICES ? "通知" : "この話の発言"}
      tabindex="0"
      onScroll={(e) => {
        const { scrollTop: top, scrollHeight: height, clientHeight: room } = e.currentTarget;
        if (height - top - room < 40) follows = true;
        else if (top < was.top && height === was.height && room === was.room) follows = false;
        was = { top, height, room };
      }}
    >
      <div class="said" ref={said}>
        <Show when={props.talk.failed()}>{(error) => <ErrorState error={error()} onRetry={() => void props.talk.retry()} />}</Show>
        <Show when={!props.talk.there() && props.talk.messages().length === 0}>
          <Loading />
        </Show>
        <Show when={props.talk.more()}>
          <Button small onClick={readBack}>
            前の通知も見る
          </Button>
        </Show>
        <For each={props.talk.messages()}>
          {(m) => (
            <>
              <Show when={m.id === marked()}>
                <div class="talk-fresh" role="separator">
                  ここから新しい
                </div>
              </Show>
              <Message m={m} aide={props.aide} tz={props.tz} quote={m.replyTo} onReply={props.onReply} onQuote={goTo} />
            </>
          )}
        </For>
        <Show when={props.working}>
          <div class="working" data-testid="working">
            <span class="working-dot" aria-hidden="true" />
            <span class="working-text">{props.working}</span>
          </div>
        </Show>
      </div>
    </div>
  );
}
