import { createMemo, createSignal, For, Show } from "solid-js";
import AideIcon from "~/components/AideIcon";
import { Button, EmptyState, ErrorState, Keyed, Loading, Tag } from "~/components/ui";
import { type Conversation, Speaker } from "~/gen/famulen/v1/console_pb";
import { clock, dayIn, shortTime, today } from "~/lib/dates";
import * as icons from "~/lib/icons";
import { excerptOf } from "~/lib/records";
import { type createTalks, go, openTalk, SOURCES, titleOf } from "~/lib/talks";
import { useWorkgroups } from "~/lib/workgroups";

export type Waits = { approvals: number; held: number };

const GROUPS = ["あなたの答え待ち", "新しく届いた", "今日", "以前"] as const;

type Kept = "" | "ask" | "unread" | "busy";

/** One conversation: what it is called, what was said last, and what it waits for. */
function Row(props: { talk: Conversation; waits: Waits; aide: string; tz: string; today: string }) {
  const c = () => props.talk;
  const workgroups = useWorkgroups();
  const workgroup = () => workgroups()?.find((w) => w.id === c().workgroup && w.id !== "all")?.name;
  const last = () => c().last;
  return (
    <li class="item picks talk-row" classList={{ unread: c().unread > 0 }}>
      <div class="item-main">
        <div class="talk-line">
          <button type="button" class="item-title item-pick talk-title" onClick={() => openTalk(c().id)}>
            {titleOf(c())}
          </button>
          <span class="talk-at">{at(c().usedAt, props.tz, props.today)}</span>
        </div>
        <div class="talk-line">
          <span class="talk-last">
            <Show when={last()} fallback="まだ発言はない">
              {(m) => `${m().who === Speaker.OWNER ? "あなた" : props.aide}：${excerptOf(m().text, 80) || m().attachments.map((f) => f.name).join("、")}`}
            </Show>
          </span>
          <Show when={c().unread > 0}>
            <span class="talk-count" aria-label={`未読 ${c().unread}`}>
              {c().unread}
            </span>
          </Show>
        </div>
        <div class="item-sub">
          <Show when={props.waits.approvals > 0}>
            <Tag tone="warn" icon={icons.handStop}>
              答え待ち {props.waits.approvals}
            </Tag>
          </Show>
          <Show when={props.waits.held > 0}>
            <Tag tone="warn" icon={icons.alertTriangle}>
              途中で切れた
            </Tag>
          </Show>
          <Show when={c().running}>
            <Tag tone="info" icon={icons.refresh}>
              考えている
            </Tag>
          </Show>
          <Show when={!c().running && c().waiting}>
            <Tag icon={icons.clock}>順番待ち</Tag>
          </Show>
          <Show when={workgroup()}>{(name) => <span>{name()}</span>}</Show>
          <Show when={SOURCES[c().source]}>{(from) => <span>{from()}</span>}</Show>
        </div>
      </div>
    </li>
  );
}

/** A time of today, or the day before it. */
const at = (when: bigint, tz: string, now: string) => (dayIn(when, tz) === now ? clock(when, tz) : shortTime(when, tz));

/** The secretary's conversations, those that wait for the owner first. */
export default function TalkList(props: { talks: ReturnType<typeof createTalks>; waits: (id: bigint) => Waits; aide: string; tz: string }) {
  const [kept, setKept] = createSignal<Kept>("");
  const all = () => props.talks.talks();
  const asks = (c: Conversation) => {
    const w = props.waits(c.id);
    return w.approvals + w.held > 0;
  };
  const is: { [K in Kept]: (c: Conversation) => boolean } = {
    "": () => true,
    ask: asks,
    unread: (c) => c.unread > 0,
    busy: (c) => c.running || c.waiting,
  };
  const now = () => today(props.tz);
  const FILTERS: [Kept, string][] = [
    ["", "すべて"],
    ["ask", "答え待ち"],
    ["unread", "未読"],
    ["busy", "動いている"],
  ];
  const groups = createMemo<{ [G in (typeof GROUPS)[number]]: Conversation[] }>(() => {
    const shown = all().filter(is[kept()]);
    const rest = shown.filter((c) => !asks(c) && c.unread === 0);
    const isToday = (c: Conversation) => dayIn(c.usedAt, props.tz) === now();
    return {
      あなたの答え待ち: shown.filter(asks),
      新しく届いた: shown.filter((c) => !asks(c) && c.unread > 0),
      今日: rest.filter(isToday),
      以前: rest.filter((c) => !isToday(c)),
    };
  });
  return (
    <div class="talk-list" data-testid="talks" tabindex="0" role="region" aria-label="話の一覧">
      <div class="talk-tools">
        <AideIcon size={28} />
        <b class="grow">{props.aide}との話</b>
        <Button small variant="ghost" icon={icons.messages} onClick={() => go({ kind: "all" })}>
          一続きで見る
        </Button>
        <Button small icon={icons.messagePlus} onClick={() => go({ kind: "new" })}>
          新しい話
        </Button>
      </div>
      <div class="chips-line" role="group" aria-label="話を絞る">
        <For each={FILTERS}>
          {([k, name]) => (
            <button type="button" class="tag label" aria-pressed={kept() === k} onClick={() => setKept(k)}>
              {name} {all().filter(is[k]).length}
            </button>
          )}
        </For>
      </div>
      <Show when={props.talks.failed()}>{(error) => <ErrorState error={error()} onRetry={props.talks.retry} />}</Show>
      <Show when={props.talks.there()} fallback={<Show when={!props.talks.failed()}><Loading /></Show>}>
        <Show when={GROUPS.every((g) => groups()[g].length === 0)}>
          <EmptyState>{all().length > 0 ? "当てはまる話はない" : `${props.aide} に頼みごとを書くと、話が始まる。右の画面で見ているものも一緒に渡る。`}</EmptyState>
        </Show>
        <For each={GROUPS}>
          {(g) => (
            <Show when={groups()[g].length > 0}>
              <section class="stack" aria-label={g}>
                <div class="row">
                  <h2>{g}</h2>
                  <span class="sub">{groups()[g].length}</span>
                </div>
                <ul class="list">
                  <Keyed each={groups()[g]} by={(c) => c.id}>
                    {(c) => <Row talk={c()} waits={props.waits(c().id)} aide={props.aide} tz={props.tz} today={now()} />}
                  </Keyed>
                </ul>
              </section>
            </Show>
          )}
        </For>
        <Show when={props.talks.more()}>
          <Button small onClick={() => props.talks.further()}>
            前の話も見る
          </Button>
        </Show>
      </Show>
    </div>
  );
}
