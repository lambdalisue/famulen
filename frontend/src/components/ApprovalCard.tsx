import { createAsync } from "@solidjs/router";
import { createSignal, For, onMount, Show } from "solid-js";
import AideIcon from "~/components/AideIcon";
import DefinitionDiff from "~/components/DefinitionDiff";
import Markdown from "~/components/Markdown";
import Questions from "~/components/Questions";
import { type Approval, ApprovalAnswer } from "~/gen/famulen/v1/console_pb";
import { askedOf, type Given, givenOf } from "~/lib/approvals";
import { chatMarkdown } from "~/lib/chat";
import { connectionsQuery } from "~/lib/connections";
import { Button, Disclosure } from "~/components/ui";

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

/** What each held call does and where, by its tool. */
export function useAsked() {
  const connections = createAsync(() => connectionsQuery());
  return (tool: string) => askedOf(tool, connections()?.connections ?? []);
}

/** A long text, the first of it until the rest is asked for. */
function LongText(props: { text: string; code: boolean }) {
  const [open, setOpen] = createSignal(false);
  const [cut, setCut] = createSignal(false);
  let el: HTMLDivElement | undefined;
  onMount(() => setCut(!!el && el.scrollHeight > el.clientHeight + 4));
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

function Rows(props: { rows: Given[] }) {
  return (
    <dl class="asked-rows">
      <For each={props.rows}>
        {(row) => (
          <div classList={{ long: row.lines || row.rows.length > 0 }}>
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

/** A call that waits for the owner's answer. */
export default function ApprovalCard(props: { approval: Approval; onAnswer: (answer: ApprovalAnswer, replies?: string[]) => void }) {
  const r = () => props.approval;
  const asked = useAsked();
  const call = () => asked(r().tool);
  const change = () => (r().tool === CHANGE_DEFINITION ? proposal(r().input) : undefined);
  const asks = () => r().questions.length > 0;
  return (
    <div class="approval" data-testid="approval" data-kind={change() ? "definition" : undefined}>
      <div class="asked-head">
        <AideIcon size={28} />
        <div class="grow">
          <b class="asked-does">{call().does}</b>
          <span class="sub">
            確認 #{r().id.toString()} · {change()?.definition || call().where}
          </span>
        </div>
      </div>
      <Show when={change()?.why || r().summary}>
        {(why) => (
          <div class="asked">
            <Markdown text={chatMarkdown(why())} plain={why()} />
          </div>
        )}
      </Show>
      <Show
        when={change()}
        fallback={
          <Show
            when={asks()}
            fallback={
              <>
                <Show when={givenOf(r().input)} fallback={<pre>{pretty(r().input)}</pre>}>
                  {(rows) => <Rows rows={rows()} />}
                </Show>
                <Whole approval={r()} />
              </>
            }
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
          <Button variant="go" small onClick={() => props.onAnswer(ApprovalAnswer.ALLOW)}>
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
  );
}
