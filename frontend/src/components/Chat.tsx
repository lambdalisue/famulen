import { revalidate } from "@solidjs/router";
import AideIcon from "~/components/AideIcon";
import ApprovalCard from "~/components/ApprovalCard";
import { DraftList } from "~/components/Attachments";
import Message, { Quote } from "~/components/Message";
import { createPull } from "~/components/pull";
import Thread from "~/components/Thread";
import * as icons from "~/lib/icons";
import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";
import { ApprovalAnswer, ApprovalSchema, type ConsoleMessage, HeldInputSchema, Speaker } from "~/gen/famulen/v1/console_pb";
import { message } from "~/lib/act";
import { api } from "~/lib/api";
import { ACCEPTED, createDrafts } from "~/lib/attachments";
import { workgroup } from "~/lib/workgroup";
import { createConversation } from "~/lib/conversation";
import { inboxQuery } from "~/lib/data";
import { useWaiting } from "~/lib/inbox";
import { working } from "~/lib/events";
import { createKept, createSame } from "~/lib/kept";
import { excerptOf } from "~/lib/records";
import { offsetIn, pointOut } from "~/lib/scroll";
import { createThread } from "~/lib/thread";
import { useOwner } from "~/lib/owner";
import { isNarrow } from "~/lib/viewport";
import { selection, setSelection, view } from "~/lib/view";
import { Button, Callout, EmptyState, ErrorState, Icon, IconButton, Tag } from "~/components/ui";
import { createResizer } from "~/components/ui/resize";

function Inbox(props: { aide: string }) {
  const inbox = createKept(() => inboxQuery());
  const heldOnes = createSame(HeldInputSchema, () => inbox()?.held ?? [], (h) => h.id);
  const approvals = createSame(ApprovalSchema, () => inbox()?.approvals ?? [], (r) => r.id);
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
      <For each={heldOnes()}>
        {(h) => (
          <div class="approval">
            <header class="asked-head">
              <Icon svg={icons.alertTriangle} size={16} />
              <b>途中で切れた依頼</b>
            </header>
            <div class="asked-in">
              <div class="asked">{[...h.text].slice(0, 300).join("")}</div>
              <p class="sub">
                {props.aide}{" "}
                が止まったとき、外への呼び出しがまだ終わっていなかった。もう一度やると、送信などが二重になるかもしれない。済んでいたかを確かめてから選ぶ。
              </p>
              <div class="row">
                <Button variant="primary" small onClick={() => held(h.id, true)}>
                  続ける
                </Button>
                <Button small onClick={() => held(h.id, false)}>
                  やめる
                </Button>
              </div>
            </div>
          </div>
        )}
      </For>
      <For each={approvals()}>
        {(r) => <ApprovalCard approval={r} onAnswer={(answer, replies) => void approve(r.id, answer, replies)} />}
      </For>
    </div>
  );
}

/** What stands before the oldest message read: that more is coming, why it did not, or that it is the first. */
function Before(props: { talk: ReturnType<typeof createConversation> }) {
  return (
    <div class="said-before" data-testid="said-before">
      <Show when={!props.talk.failed()} fallback={<ErrorState error={props.talk.failed()} onRetry={() => void props.talk.retry()} />}>
        {props.talk.more() ? "前の発言を読み込んでいる" : "会話の最初です"}
      </Show>
    </div>
  );
}

const CHAT_WIDTH = 460;
/** The least the page beside the conversation is left with. */
const MAIN_LEAST = 560;

/** The conversation column. It stays mounted while pages change. */
export default function Chat(props: { open: boolean; setOpen: (open: boolean) => void }) {
  const owner = useOwner();
  let follows = true;
  /** An older message is read back to: nothing is followed until it is there. */
  let seeking = false;
  const talk = createConversation(workgroup, () => follows && !seeking);
  const messages = talk.messages;
  /** The message the open thread begins with. */
  const [root, setRoot] = createSignal<bigint>();
  const thread = createThread(workgroup, root, messages);
  /** How many replies each thread has. All of them are read once the message it begins with is. */
  const replies = createMemo(() => {
    const by = new Map<bigint, number>();
    for (const m of messages()) if (m.thread) by.set(m.thread, (by.get(m.thread) ?? 0) + 1);
    return by;
  });
  /** The message the owner picked to reply to. */
  const [picked, setPicked] = createSignal<ConsoleMessage>();
  const replyTo = () => picked() ?? thread.messages()[0];
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
  const last = () => [...messages()].reverse().find((m) => m.who === Speaker.AIDE);
  const waiting = useWaiting();

  // The newest stays in sight, whatever grows or shrinks, until the owner
  // scrolls up to read what came before. Then what is read stays where it is:
  // the browser does not see to it, as Safari cannot.
  let was = { top: 0, height: 0, room: 0 };
  /** The message at the top of what is seen and the part of it there, and how far below the top each begins. */
  let reading: { id: string; at: number; part: Element; partAt: number } | undefined;
  const offset = (message: Element, seen: HTMLElement) => message.getBoundingClientRect().top - seen.getBoundingClientRect().top;
  const mark = (seen: HTMLElement) => {
    const top = seen.getBoundingClientRect().top;
    const below = (el: Element) => el.getBoundingClientRect().bottom > top;
    const first = [...seen.querySelectorAll<HTMLElement>("[data-id]")].find(below);
    const part = first && ([...first.querySelectorAll(".markdown-body > *, .attached > *, .link-cards > *")].find(below) ?? first);
    reading = first?.dataset.id && part ? { id: first.dataset.id, at: offset(first, seen), part, partAt: offset(part, seen) } : undefined;
    was = { top: seen.scrollTop, height: seen.scrollHeight, room: seen.clientHeight };
  };
  /** Put right what grew or shrank above what is read, but for how far the owner scrolled since. */
  const hold = (seen: HTMLElement) => {
    if (!reading) return;
    // What is drawn anew, as once its files came in, is found again by its message.
    const { part, partAt, id, at } = reading;
    const message = part.isConnected ? part : seen.querySelector(`[data-id="${id}"]`);
    if (!message) return;
    const by = offset(message, seen) - ((part.isConnected ? partAt : at) - (seen.scrollTop - was.top));
    if (Math.abs(by) >= 1) seen.scrollTop += by;
  };
  // What came before is read a screen ahead of where the owner is.
  const readOn = (seen: HTMLElement) => {
    if (seen.scrollTop < seen.clientHeight) void talk.older();
  };
  const onScroll = (e: Event & { currentTarget: HTMLElement }) => {
    const seen = e.currentTarget;
    const { scrollTop: top, scrollHeight: height, clientHeight: room } = seen;
    if (room === 0) return;
    if (height - top - room < 40) follows = !seeking;
    else if (top < was.top && height === was.height && room === was.room) follows = false;
    if (!follows && height !== was.height) hold(seen);
    mark(seen);
    readOn(seen);
  };
  const toBottom = () => {
    follows = true;
    if (list) list.scrollTop = list.scrollHeight;
  };
  onMount(() => {
    if (!list || !said) return;
    const seen = list;
    const sized = new ResizeObserver(() => {
      if (seen.clientHeight === 0) return;
      if (follows) seen.scrollTop = seen.scrollHeight;
      else hold(seen);
      mark(seen);
      readOn(seen);
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
        // Folded, nothing would show that what is written goes into the thread.
        if (!open && isNarrow()) setRoot();
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
  /** Counts the times a message was gone to, so that only the latest is followed through. */
  let gone = 0;
  createEffect(
    on(
      talk.workgroup,
      () => {
        gone += 1;
        seeking = false;
        toBottom();
      },
      { defer: true },
    ),
  );

  const reply = (m: ConsoleMessage) => {
    if ((m.thread || m.id) !== root()) setRoot();
    setPicked(m);
    say?.focus();
  };
  const openThread = (id: bigint | undefined) => {
    setPicked();
    setRoot(id);
  };
  createEffect(on(workgroup, () => openThread(undefined), { defer: true }));
  /** Go to a message, reading back to it when it is older than what is read. */
  const goTo = async (id: bigint) => {
    const mine = ++gone;
    follows = false;
    seeking = true;
    const found = await talk.reach(id);
    if (mine !== gone || !list) return;
    seeking = false;
    const to = list.querySelector<HTMLElement>(`[data-id="${id}"]`);
    if (found && to) {
      list.scrollTop += offsetIn(list, to);
      pointOut(to);
    } else setError("元の発言を読み込めなかった");
    // Near the end nothing may have scrolled to tell.
    follows = list.scrollHeight - list.scrollTop - list.clientHeight < 40;
    mark(list);
  };

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
    const of = workgroup();
    const target = picked();
    const to = replyTo()?.id ?? root();
    const within = root();
    try {
      const m = await api.console.send({
        text: body,
        workgroup: of,
        view: view(),
        selection: selection(),
        attachments: files.flatMap((f) => f.id ?? []),
        replyTo: to ?? 0n,
      });
      setText("");
      attached.remove(files);
      setSelection("");
      if (picked() === target) setPicked();
      talk.sent(of, m);
      // Within a thread, what is read beside it stays.
      if (within === undefined) toBottom();
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
        <div class="talk">
          <div id="messages" class="messages" ref={list} role="log" aria-label="会話" aria-busy={talk.reading()} tabindex="0" onScroll={onScroll}>
            <div class="said" ref={said}>
              <Show when={messages().length === 0 && !talk.more()}>
                <EmptyState>{aide()} に頼みごとを書く。右の画面で見ているものも一緒に渡る。</EmptyState>
              </Show>
              <Show when={messages().length > 0 || talk.more()}>
                <Before talk={talk} />
              </Show>
              <For each={messages()}>{(m) => <Message m={m} aide={aide()} tz={tz()} quote={m.replyTo} onReply={reply} replies={replies().get(m.id)} onQuote={(id) => void goTo(id)} onThread={openThread} />}</For>
              <Show when={working()}>
                <div class="working" data-testid="working">
                  <span class="working-dot" aria-hidden="true" />
                  <span class="working-text">{working()}</span>
                </div>
              </Show>
            </div>
          </div>
          <Show when={root()} keyed>
            {(id) => <Thread thread={thread} root={id} aide={aide()} tz={tz()} onReply={reply} onClose={() => openThread(undefined)} />}
          </Show>
        </div>
        <Inbox aide={aide()} />
        <button type="button" class="m-peek" aria-label="会話を引き上げる" onClick={() => props.setOpen(true)}>
          <span class="sheet-grip" />
          <span class="peek-line">
            <AideIcon size={24} />
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
          <Show when={replyTo()}>
            {(m) => (
              <div class="replying" data-testid="replying">
                <Icon svg={icons.arrowBackUp} size={14} />
                <Quote of={{ who: m().who, text: m().text || m().attachments.map((f) => f.name).join("、") }} aide={aide()} />
                <Show when={picked()}>
                  <IconButton icon={icons.x} label="返信をやめる" variant="ghost" onMouseDown={(e) => e.preventDefault()} onClick={() => setPicked()} />
                </Show>
              </div>
            )}
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
