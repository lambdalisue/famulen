import { shortTime } from "~/lib/dates";
import { A, createAsync, revalidate } from "@solidjs/router";
import AideIcon from "~/components/AideIcon";
import ApprovalCard from "~/components/ApprovalCard";
import { AttachedList, DraftList } from "~/components/Attachments";
import Markdown from "~/components/Markdown";
import { createPull } from "~/components/pull";
import * as icons from "~/lib/icons";
import { createEffect, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";
import { ApprovalAnswer, type ConsoleMessage, Speaker } from "~/gen/famulen/v1/console_pb";
import { message } from "~/lib/act";
import { api } from "~/lib/api";
import { ACCEPTED, createDrafts } from "~/lib/attachments";
import { chatMarkdown } from "~/lib/chat";
import { workgroup } from "~/lib/workgroup";
import { inboxQuery, messagesQuery } from "~/lib/data";
import { useWaiting } from "~/lib/inbox";
import { pushed, working } from "~/lib/events";
import { excerptOf } from "~/lib/records";
import { tabNamed } from "~/lib/tabs";
import { useOwner } from "~/lib/owner";
import { selection, setSelection, view } from "~/lib/view";
import { Button, Callout, EmptyState, Icon, IconButton, LinkButton, Tag } from "~/components/ui";
import { createResizer } from "~/components/ui/resize";

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
      <Show when={props.m.text}>
        <div class="text">
          <Markdown text={chatMarkdown(props.m.text)} plain={props.m.text} />
        </div>
      </Show>
      <AttachedList files={props.m.attachments} at={props.m.at} />
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
        {(r) => <ApprovalCard approval={r} onAnswer={(answer, replies) => void approve(r.id, answer, replies)} />}
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
  const attached = createDrafts();
  const [dropping, setDropping] = createSignal(false);
  let picker: HTMLInputElement | undefined;
  let list: HTMLDivElement | undefined;
  let said: HTMLDivElement | undefined;
  let say: HTMLTextAreaElement | undefined;

  const aide = owner.aide;
  const tz = owner.tz;
  const messages = () => {
    // One pushed again is how it stands now, such as once its files came in.
    const said = new Map<bigint, ConsoleMessage>();
    for (const m of [...(loaded() ?? []), ...extra()]) said.set(m.id, m);
    return [...said.values()].sort((a, b) => Number(a.id - b.id));
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
      (open) => {
        // A tap on a button leaves the focus where it was on iOS, and with it the keyboard.
        if (open) toBottom();
        else say?.blur();
      },
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
  // What floats over the page on a phone stands on the composer, which grows with what goes along.
  onMount(() => {
    if (!column) return;
    const grown = new ResizeObserver(() => {
      const height = column?.offsetHeight;
      if (height && !props.open) document.body.style.setProperty("--composer", `${height}px`);
    });
    grown.observe(column);
    onCleanup(() => {
      grown.disconnect();
      document.body.style.removeProperty("--composer");
    });
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
        setExtra((e) => [...e.filter((x) => x.id !== m.id), m]);
      },
      { defer: true },
    ),
  );

  const send = async (e?: Event) => {
    e?.preventDefault();
    const body = text().trim();
    const files = attached.drafts();
    if (!body && files.length === 0) return;
    if (files.some((f) => f.problem)) {
      setError("届いていないファイルがある。外すか、添え直してから送る");
      return;
    }
    if (files.some((f) => !f.id)) {
      setError("ファイルを送っている。届いてから送る");
      return;
    }
    setError("");
    try {
      const m = await api.console.send({
        text: body,
        workgroup: workgroup(),
        view: view(),
        selection: selection(),
        attachments: files.flatMap((f) => f.id ?? []),
      });
      setText("");
      attached.remove(files);
      setSelection("");
      setExtra((x) => (x.some((y) => y.id === m.id) ? x : [...x, m]));
      toBottom();
    } catch (err) {
      setError(message(err));
    }
  };

  const attach = (files: Iterable<File>) => {
    setError("");
    attached.add(files);
  };
  const carriesFiles = (e: DragEvent) => e.dataTransfer?.types.includes("Files") ?? false;

  return (
    <>
      <aside
        class="chat"
        classList={{ dropping: dropping() }}
        aria-label={`${aide()} との会話`}
        ref={column}
        style={{ "--chat": `${size.width()}px` }}
        onDragOver={(e) => {
          if (!carriesFiles(e)) return;
          e.preventDefault();
          setDropping(true);
        }}
        onDragLeave={(e) => {
          if (!(e.relatedTarget instanceof Node && e.currentTarget.contains(e.relatedTarget))) setDropping(false);
        }}
        onDrop={(e) => {
          setDropping(false);
          if (!carriesFiles(e)) return;
          e.preventDefault();
          attach(e.dataTransfer?.files ?? []);
        }}
        {...pull}
      >
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
            <Show when={working()}>
              <div class="working" data-testid="working">
                <span class="working-dot" aria-hidden="true" />
                <span class="working-text">{working()}</span>
              </div>
            </Show>
          </div>
        </div>
        <Inbox aide={aide()} />
        <button type="button" class="m-peek" aria-label="会話を引き上げる" onClick={() => props.setOpen(true)}>
          <span class="sheet-grip" />
          <span class="peek-line">
            <span class="peek-text">{working() || excerptOf(last()?.text ?? "", 200) || "まだ話していない"}</span>
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
          <DraftList
            drafts={attached.drafts()}
            onRemove={(d) => {
              setError("");
              attached.remove([d]);
            }}
          />
          <div class="line">
            <input
              ref={picker}
              type="file"
              multiple
              hidden
              accept={ACCEPTED}
              data-testid="picker"
              onChange={(e) => {
                attach([...(e.currentTarget.files ?? [])]);
                // So that the same file can be chosen again.
                e.currentTarget.value = "";
              }}
            />
            <IconButton
              icon={icons.paperclip}
              label="ファイルを添える"
              variant="ghost"
              class="attach"
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => picker?.click()}
            />
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
              onPaste={(e) => {
                const files = [...(e.clipboardData?.files ?? [])];
                // Copied cells and the like bring a picture of themselves beside their text.
                const types = e.clipboardData?.types ?? [];
                if (files.length === 0 || ["text/plain", "text/html"].every((t) => types.includes(t))) return;
                e.preventDefault();
                attach(files);
              }}
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
      <div class="chat-edge">
        <div class="resize-edge" aria-valuenow={size.width()} {...size.separator} />
      </div>
    </>
  );
}
