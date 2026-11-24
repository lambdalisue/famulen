import { shortTime } from "~/lib/dates";
import { A, createAsync, revalidate } from "@solidjs/router";
import AideIcon from "~/components/AideIcon";
import DefinitionDiff from "~/components/DefinitionDiff";
import Markdown from "~/components/Markdown";
import { createPull } from "~/components/pull";
import Questions from "~/components/Questions";
import * as icons from "~/lib/icons";
import { createEffect, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";
import { ApprovalAnswer, type ConsoleMessage, Speaker } from "~/gen/famulen/v1/console_pb";
import { message } from "~/lib/act";
import { api } from "~/lib/api";
import { chatMarkdown } from "~/lib/chat";
import { workgroup } from "~/lib/workgroup";
import { inboxQuery, messagesQuery } from "~/lib/data";
import { useWaiting } from "~/lib/inbox";
import { pushed } from "~/lib/events";
import { excerptOf } from "~/lib/records";
import { tabNamed } from "~/lib/tabs";
import { useOwner } from "~/lib/owner";
import { selection, setSelection, view } from "~/lib/view";
import { Button, Callout, EmptyState, Icon, IconButton, LinkButton, Tag } from "~/components/ui";
import { createResizer } from "~/components/ui/resize";

/** How the gateway names a held change to a service definition. */
const CHANGE_DEFINITION = "definitions.change_definition";

type Proposal = { definition: string; toml: string; why: string; before: string };

/** A held change to a service definition, from its input. */
function proposal(input: string): Proposal | undefined {
  try {
    const v: unknown = JSON.parse(input);
    if (typeof v !== "object" || v === null) return undefined;
    const field = (name: string) => {
      const f: unknown = Reflect.get(v, name);
      return typeof f === "string" ? f : "";
    };
    return { definition: field("definition"), toml: field("toml"), why: field("why"), before: field("before") };
  } catch {
    return undefined;
  }
}

function pretty(input: string): string {
  try {
    return JSON.stringify(JSON.parse(input), null, 2);
  } catch {
    return input;
  }
}

function Message(props: { m: ConsoleMessage; aide: string; tz: string }) {
  const owner = () => props.m.who === Speaker.OWNER;
  const open = () => tabNamed(props.m.open);
  return (
    <div class={owner() ? "msg owner" : "msg aide"} data-testid="message">
      <span class="meta">
        <Show when={!owner()}>
          <AideIcon size={20} />
        </Show>
        <span>
          {owner() ? "あなた" : props.aide} · {shortTime(props.m.at, props.tz)}
          {props.m.origin ? ` · ${props.m.origin}` : ""}
          {props.m.about ? ` · ${props.m.about}` : ""}
        </span>
      </span>
      <div class="text">
        <Markdown text={chatMarkdown(props.m.text)} plain={props.m.text} />
      </div>
      <Show when={open()}>
        {(tab) => (
          <LinkButton small icon={tab().icon} href={tab().path}>
            {tab().label}で見る
          </LinkButton>
        )}
      </Show>
    </div>
  );
}

function Inbox(props: { aide: string }) {
  const inbox = createAsync(() => inboxQuery());
  const approve = async (id: bigint, answer: ApprovalAnswer, replies: string[] = []) => {
    await api.console.answerApproval({ id, answer, replies });
    await revalidate("inbox");
  };
  const held = async (id: bigint, goOn: boolean) => {
    await api.console.answerHeld({ id, goOn });
    await revalidate("inbox");
  };
  return (
    <div id="approvals">
      <For each={inbox()?.held}>
        {(h) => (
          <div class="approval">
            <div class="row">
              <AideIcon size={20} />
              <b class="approval-title">途中で切れた依頼</b>
            </div>
            <div class="asked">{[...h.text].slice(0, 300).join("")}</div>
            <p class="sub">
              {props.aide}{" "}
              が止まったとき、外への呼び出しがまだ終わっていなかった。もう一度やると、送信などが二重になるかもしれない。済んでいたかを確かめてから選ぶ。
            </p>
            <div class="row">
              <Button variant="go" small onClick={() => held(h.id, true)}>
                続ける
              </Button>
              <Button small onClick={() => held(h.id, false)}>
                やめる
              </Button>
            </div>
          </div>
        )}
      </For>
      <For each={inbox()?.approvals}>
        {(r) => (
          <Show
            when={r.tool === CHANGE_DEFINITION && proposal(r.input)}
            fallback={
              <div class="approval" data-testid="approval">
                <div class="row">
                  <AideIcon size={20} />
                  <b class="approval-title">確認 #{r.id.toString()}</b>
                  <span class="mono sub">{r.tool}</span>
                </div>
                <Show when={r.summary}>
                  <p class="asked">{r.summary}</p>
                </Show>
                <Show
                  when={r.questions.length === 0}
                  fallback={<Questions questions={r.questions} onAnswer={(replies) => approve(r.id, ApprovalAnswer.ALLOW, replies)} />}
                >
                  <pre>{pretty(r.input)}</pre>
                </Show>
                <div class="row">
                  <Show when={r.questions.length === 0}>
                    <Button variant="go" small onClick={() => approve(r.id, ApprovalAnswer.ALLOW)}>
                      承認
                    </Button>
                  </Show>
                  <Button small onClick={() => approve(r.id, ApprovalAnswer.DENY)}>
                    {r.questions.length === 0 ? "却下" : "ここでは答えない"}
                  </Button>
                  <Show when={!r.once}>
                    <Button small onClick={() => approve(r.id, ApprovalAnswer.ALWAYS)}>
                      今後は常に許可
                    </Button>
                  </Show>
                </div>
              </div>
            }
          >
            {(p) => (
              <div class="approval" data-testid="approval" data-kind="definition">
                <div class="row">
                  <AideIcon size={20} />
                  <b class="approval-title">確認 #{r.id.toString()}</b>
                  <span>
                    サービス定義 <span class="mono">{p().definition}</span> を変える
                  </span>
                </div>
                <Show when={p().why}>
                  <p class="asked">{p().why}</p>
                </Show>
                <DefinitionDiff before={p().before} after={p().toml} />
                <p class="sub">
                  許すと定義に書き込み、つないでいるサービスの道具がこのとおりになる。足された書き込みの道具は、確かめてから使う。
                </p>
                <div class="row">
                  <Button variant="go" small onClick={() => approve(r.id, ApprovalAnswer.ALLOW)}>
                    許す
                  </Button>
                  <Button small onClick={() => approve(r.id, ApprovalAnswer.DENY)}>
                    断る
                  </Button>
                </div>
              </div>
            )}
          </Show>
        )}
      </For>
    </div>
  );
}

const CHAT_WIDTH = 460;
/** The least the page beside the conversation is left with. */
const MAIN_LEAST = 560;

/** The conversation column. It stays mounted while pages change. */
export default function Chat(props: { open: boolean; setOpen: (open: boolean) => void }) {
  const owner = useOwner();
  const loaded = createAsync(() => messagesQuery(workgroup()));
  const [extra, setExtra] = createSignal<ConsoleMessage[]>([]);
  const [text, setText] = createSignal("");
  const [error, setError] = createSignal("");
  let list: HTMLDivElement | undefined;
  let said: HTMLDivElement | undefined;
  let say: HTMLTextAreaElement | undefined;

  const aide = owner.aide;
  const tz = owner.tz;
  const messages = () => {
    const seen = new Set((loaded() ?? []).map((m) => m.id));
    return [...(loaded() ?? []), ...extra().filter((m) => !seen.has(m.id))];
  };
  const last = () => [...messages()].reverse().find((m) => m.who === Speaker.AIDE);
  const waiting = useWaiting();

  // The newest stays in sight, whatever grows or shrinks, until the owner
  // scrolls up to read what came before.
  let follows = true;
  let was = { top: 0, height: 0, room: 0 };
  const onScroll = (e: Event & { currentTarget: HTMLElement }) => {
    const { scrollTop: top, scrollHeight: height, clientHeight: room } = e.currentTarget;
    if (height - top - room < 40) follows = true;
    else if (top < was.top && height === was.height && room === was.room) follows = false;
    was = { top, height, room };
  };
  const toBottom = () => {
    follows = true;
    if (list) list.scrollTop = list.scrollHeight;
  };
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
      () => props.open,
      (open) => open && toBottom(),
      { defer: true },
    ),
  );

  let column: HTMLElement | undefined;
  const size = createResizer({
    key: "famulen.chat",
    width: CHAT_WIDTH,
    least: 320,
    most: () => window.innerWidth - MAIN_LEAST,
    from: (e) => e.clientX - (column?.getBoundingClientRect().left ?? 0),
    toward: "ArrowRight",
  });
  const pull = createPull({
    sheet: () => column,
    open: () => props.open,
    setOpen: (open) => props.setOpen(open),
  });
  createEffect(
    on(
      loaded,
      () => {
        setExtra([]);
        toBottom();
      },
      { defer: true },
    ),
  );
  createEffect(
    on(
      pushed,
      (m) => {
        if (!m) return;
        setExtra((e) => (e.some((x) => x.id === m.id) ? e : [...e, m]));
      },
      { defer: true },
    ),
  );

  const send = async (e?: Event) => {
    e?.preventDefault();
    const body = text().trim();
    if (!body) return;
    setError("");
    try {
      const m = await api.console.send({ text: body, workgroup: workgroup(), view: view(), selection: selection() });
      setText("");
      setSelection("");
      setExtra((x) => (x.some((y) => y.id === m.id) ? x : [...x, m]));
      toBottom();
    } catch (err) {
      setError(message(err));
    }
  };

  return (
    <aside class="chat" aria-label={`${aide()} との会話`} ref={column} style={{ "--chat": `${size.width()}px` }} {...pull}>
      <div class="resize-edge chat-edge" aria-valuenow={size.width()} {...size.separator} />
      <div class="m-sheetbar">
        <div class="sheet-grip" />
        <div class="row">
          <AideIcon size={28} />
          <b class="grow">{aide()}との会話</b>
          <IconButton icon={icons.chevronDown} label="閉じる" variant="ghost" onClick={() => props.setOpen(false)} />
        </div>
      </div>
      <div id="messages" class="messages" ref={list} role="log" aria-label="会話" tabindex="0" onScroll={onScroll}>
        <div class="said" ref={said}>
          <Show when={messages().length === 0}>
            <EmptyState>{aide()} に頼みごとを書く。右の画面で見ているものも一緒に渡る。</EmptyState>
          </Show>
          <For each={messages()}>{(m) => <Message m={m} aide={aide()} tz={tz()} />}</For>
        </div>
      </div>
      <Inbox aide={aide()} />
      <button type="button" class="m-peek" aria-label="会話を引き上げる" onClick={() => props.setOpen(true)}>
        <span class="sheet-grip" />
        <span class="peek-line">
          <span class="peek-text">{excerptOf(last()?.text ?? "", 200) || "まだ話していない"}</span>
          <Show when={waiting.count() > 0}>
            <Tag tone="warn">確認 {waiting.count()}</Tag>
          </Show>
        </span>
      </button>
      <form id="composer" class="composer" onSubmit={send}>
        <div class="view">
          <span class="dot" />
          <span title="この画面のことも秘書に伝わる">{view()}</span>
        </div>
        <Show when={selection()}>
          <div id="selection" class="chip">
            <span>{selection()}</span>
            <button type="button" aria-label="外す" title="添えない" onClick={() => setSelection("")}>
              <Icon svg={icons.x} size={12} />
            </button>
          </div>
        </Show>
        <div class="line">
          <button type="button" class="m-dismiss" aria-label="キーボードを閉じる" onClick={() => say?.blur()}>
            <Icon svg={icons.chevronDown} size={18} />
          </button>
          <label for="say" class="sr">
            {aide()} への頼みごと
          </label>
          <textarea
            id="say"
            ref={say}
            rows={1}
            placeholder={`${aide()}に頼む`}
            value={text()}
            onInput={(e) => setText(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void send(e);
            }}
          />
          <Button
            variant="primary"
            icon={icons.send}
            class="send"
            type="submit" // Taking focus from the box would end the narrow layout's typing
            // state and move the button out from under the tap before it lands.
            onMouseDown={(e) => e.preventDefault()}
          >
            送る
          </Button>
        </div>
        <Show when={error()}>
          <Callout tone="danger" live>
            {error()}
          </Callout>
        </Show>
      </form>
    </aside>
  );
}
