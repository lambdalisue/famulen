import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import DefinitionDiff from "~/components/DefinitionDiff";
import Markdown from "~/components/Markdown";
import Questions from "~/components/Questions";
import { type Approval, ApprovalAnswer, type ApprovalGiven } from "~/gen/famulen/v1/console_pb";
import { chatMarkdown } from "~/lib/chat";
import { approvalId } from "~/lib/inbox";
import * as icons from "~/lib/icons";
import { Button, Disclosure, Icon, SendTogether } from "~/components/ui";

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

/** A long text, the first of it until the rest is asked for. */
function LongText(props: { text: string; code: boolean }) {
  const [open, setOpen] = createSignal(false);
  const [cut, setCut] = createSignal(false);
  let el: HTMLDivElement | undefined;
  // It can be put in while the conversation is folded away, where nothing has a height.
  onMount(() => {
    if (!el) return;
    const text = el;
    const sized = new ResizeObserver(() => {
      if (!open() && text.clientHeight > 0) setCut(text.scrollHeight > text.clientHeight + 4);
    });
    sized.observe(text);
    onCleanup(() => sized.disconnect());
  });
  return (
    <>
      <div class="asked-text" classList={{ mono: props.code, open: open() }} ref={el}>
        {props.text}
      </div>
      <Show when={cut()}>
        <button type="button" class="asked-more" aria-expanded={open()} onClick={() => setOpen(!open())}>
          {open() ? "畳む" : "すべて読む"}
        </button>
      </Show>
    </>
  );
}

function Rows(props: { rows: ApprovalGiven[] }) {
  return (
    <dl class="asked-rows">
      <For each={props.rows}>
        {(row) => (
          <div classList={{ long: row.rows.length > 0 || row.text.includes("\n") || [...row.text].length > 60 }}>
            <dt>{row.name}</dt>
            <dd>
              <Show when={row.rows.length > 0} fallback={<LongText text={row.text} code={row.code} />}>
                <Rows rows={row.rows} />
              </Show>
            </dd>
          </div>
        )}
      </For>
    </dl>
  );
}

function Whole(props: { approval: Approval }) {
  return (
    <Disclosure title="元のデータ" class="asked-whole">
      <span class="mono sub">{props.approval.tool}</span>
      <pre>{pretty(props.approval.input)}</pre>
    </Disclosure>
  );
}

/** What is asked, when it is not answered by choosing: what the call is given, and the whole of it. */
function Given(props: { approval: Approval }) {
  return (
    <>
      <Show when={props.approval.given.length > 0} fallback={<pre>{pretty(props.approval.input)}</pre>}>
        <Rows rows={props.approval.given} />
      </Show>
      <Whole approval={props.approval} />
    </>
  );
}

/** What a session waits for, answered where it is read. */
export function SessionAsk(props: { approval: Approval; onAnswer: (answer: ApprovalAnswer, replies?: string[]) => unknown }) {
  return (
    <Show
      when={props.approval.questions.length > 0}
      fallback={
        <>
          <Given approval={props.approval} />
          <div class="row">
            <Button variant="primary" small onClick={() => props.onAnswer(ApprovalAnswer.ALLOW)}>
              承認
            </Button>
            <Button small onClick={() => props.onAnswer(ApprovalAnswer.DENY)}>
              却下
            </Button>
          </div>
        </>
      }
    >
      <fieldset class="asked-fields">
        <Questions questions={props.approval.questions} onAnswer={(replies) => props.onAnswer(ApprovalAnswer.ALLOW, replies)} />
      </fieldset>
    </Show>
  );
}

/** A call that waits for the owner's answer. */
export default function ApprovalCard(props: { approval: Approval; hidden?: boolean; onAnswer: (answer: ApprovalAnswer, replies?: string[]) => unknown }) {
  const r = () => props.approval;
  const change = () => (r().tool === CHANGE_DEFINITION ? proposal(r().input) : undefined);
  const asks = () => r().questions.length > 0;
  return (
    <div id={approvalId(r().id)} tabindex="-1" class="approval" hidden={props.hidden} data-testid="approval" data-kind={change() ? "definition" : undefined}>
      <header class="asked-head">
        <Icon svg={asks() ? icons.messages : icons.shieldCheck} size={16} />
        <span>
          <b>確認 #{r().id.toString()}</b> · {change()?.definition || r().place}
        </span>
      </header>
      <SendTogether hold>
        <div class="asked-in">
          <b class="asked-does">{r().deed}</b>
          <Show when={change()?.why || r().summary}>
            {(why) => (
              <div class="asked">
                <Markdown text={chatMarkdown(why())} plain={why()} />
              </div>
            )}
          </Show>
          <Show when={r().stillAsked}>{(why) => <p class="sub">{why()}</p>}</Show>
          <Show
            when={change()}
            fallback={
              <Show
                when={asks()}
                fallback={<Given approval={r()} />}
              >
                <Questions questions={r().questions} onAnswer={(replies) => props.onAnswer(ApprovalAnswer.ALLOW, replies)} />
              </Show>
            }
          >
            {(p) => (
              <>
                <DefinitionDiff before={p().before} after={p().toml} />
                <p class="sub">許すと定義に書き込み、つないでいるサービスの道具がこのとおりになる。足された書き込みの道具は、確かめてから使う。</p>
              </>
            )}
          </Show>
          <div class="row">
            <Show when={!asks()}>
              <Button variant="primary" small onClick={() => props.onAnswer(ApprovalAnswer.ALLOW)}>
                {change() ? "許す" : "承認"}
              </Button>
            </Show>
            <Button small onClick={() => props.onAnswer(ApprovalAnswer.DENY)}>
              {change() ? "断る" : asks() ? "ここでは答えない" : "却下"}
            </Button>
            <Show when={!r().once && !change()}>
              <Button small onClick={() => props.onAnswer(ApprovalAnswer.ALWAYS)}>
                今後は常に許可
              </Button>
            </Show>
          </div>
        </div>
      </SendTogether>
    </div>
  );
}
