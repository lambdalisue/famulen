import { Show } from "solid-js";
import AideIcon from "~/components/AideIcon";
import { AttachedList } from "~/components/Attachments";
import LinkCards from "~/components/LinkCards";
import Markdown from "~/components/Markdown";
import { Icon, IconButton, LinkButton } from "~/components/ui";
import { type ConsoleMessage, type RepliedTo, Speaker } from "~/gen/famulen/v1/console_pb";
import { chatMarkdown } from "~/lib/chat";
import { shortTime } from "~/lib/dates";
import * as icons from "~/lib/icons";
import { hasLinks } from "~/lib/links";
import { excerptOf } from "~/lib/records";
import { tabNamed } from "~/lib/tabs";

/** A line of the message that is replied to. */
export function Quote(props: { of: Pick<RepliedTo, "who" | "text">; aide: string }) {
  return (
    <span class="quote-line">
      <b>{props.of.who === Speaker.OWNER ? "あなた" : props.aide}</b> {excerptOf(props.of.text)}
    </span>
  );
}

/** A message, and what the owner can do with it: reply, go to what it replies to, and open its thread. */
export default function Message(props: {
  m: ConsoleMessage;
  aide: string;
  tz: string;
  /** What it replies to, when that is to be told. */
  quote: RepliedTo | undefined;
  onReply: (m: ConsoleMessage) => void;
  onQuote: (id: bigint) => void;
  /** How many replies the thread it begins has. */
  replies?: number;
  /** Left out within the thread itself. */
  onThread?: (root: bigint) => void;
}) {
  const owner = () => props.m.who === Speaker.OWNER;
  const open = () => tabNamed(props.m.open);
  const inThread = () => (props.m.thread ? "スレッドを開く" : props.replies ? `返信 ${props.replies} 件` : "");
  return (
    <div class={owner() ? "msg owner" : "msg aide"} data-testid="message" data-id={props.m.id.toString()}>
      <span class="meta">
        <Show when={!owner()}>
          <AideIcon size={28} />
        </Show>
        <span>
          <b>{owner() ? "あなた" : props.aide}</b> · {shortTime(props.m.at, props.tz)}
          {props.m.origin ? ` · ${props.m.origin}` : ""}
          {props.m.about ? ` · ${props.m.about}` : ""}
        </span>
        <IconButton icon={icons.arrowBackUp} label="返信" variant="ghost" class="reply" onMouseDown={(e) => e.preventDefault()} onClick={() => props.onReply(props.m)} />
      </span>
      <Show when={props.quote}>
        {(of) => (
          <button type="button" class="quote" title="元の発言へ" onClick={() => props.onQuote(of().id)}>
            <Quote of={of()} aide={props.aide} />
          </button>
        )}
      </Show>
      <Show when={props.m.text}>
        <div class="text">
          <Markdown text={chatMarkdown(props.m.text)} plain={props.m.text} />
        </div>
      </Show>
      <Show when={hasLinks(props.m.text)}>
        <LinkCards message={props.m.id} />
      </Show>
      <AttachedList files={props.m.attachments} at={props.m.at} />
      <Show when={open()}>
        {(tab) => (
          <LinkButton small icon={tab().icon} href={tab().path}>
            {tab().label}で見る
          </LinkButton>
        )}
      </Show>
      <Show when={props.onThread && inThread()}>
        <button type="button" class="in-thread" onClick={() => props.onThread?.(props.m.thread || props.m.id)}>
          <Icon svg={icons.messages} size={14} />
          {inThread()}
        </button>
      </Show>
    </div>
  );
}
