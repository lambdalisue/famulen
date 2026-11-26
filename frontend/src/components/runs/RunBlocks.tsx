import { createEffect, createSignal, type JSX, Match, on, Show, Switch } from "solid-js";
import Markdown from "~/components/Markdown";
import RunAsk from "~/components/records/RunAsk";
import { Disclosure, Icon, Tag } from "~/components/ui";
import { clock } from "~/lib/dates";
import * as icons from "~/lib/icons";
import { useOwner } from "~/lib/owner";
import { type Block, checksOf, durationText, KINDS, summaryOf, tokensText } from "~/lib/runview";
import type { WorkerRun } from "~/lib/workers";
import RunDiff from "./RunDiff";

type Of<K extends Block["kind"]> = Extract<Block, { kind: K }>;

/** How many lines of what a tool put out are shown before the rest is scrolled to. */
const TAIL_LINES = 400;

function At(props: { at: bigint; until?: bigint }) {
  const owner = useOwner();
  return (
    <Show when={props.at > 0n}>
      <time class="run-at" datetime={new Date(Number(props.at)).toISOString()}>
        {clock(props.at / 1000n, owner.tz())}
      </time>
    </Show>
  );
}

function tail(text: string): string {
  const lines = text.split("\n");
  return lines.length > TAIL_LINES ? `…（前の ${lines.length - TAIL_LINES} 行は略した）\n${lines.slice(-TAIL_LINES).join("\n")}` : text;
}

function Said(props: { b: Of<"said"> }) {
  return (
    <div class="run-said">
      <span class="run-mark" aria-hidden="true">
        <Icon svg={icons.message} size={15} />
      </span>
      <div class="run-said-body">
        <Show when={!props.b.live} fallback={<p class="run-live">{props.b.text}<span class="run-caret" aria-hidden="true" /></p>}>
          <Markdown text={props.b.text} />
        </Show>
        <div class="run-meta">
          <At at={props.b.at} />
          <Show when={props.b.used && props.b.used.input + props.b.used.output > 0n && props.b.used}>
            {(u) => (
              <span title={u().model}>
                <Icon svg={icons.coin} size={12} /> 入力 {tokensText(u().input)}（うちキャッシュ {tokensText(u().cached)}）・出力 {tokensText(u().output)}
              </span>
            )}
          </Show>
          <Show when={props.b.live}>
            <span class="run-writing">書いている</span>
          </Show>
        </div>
      </div>
    </div>
  );
}

function standing(b: Of<"tool">): { tone: "ok" | "danger" | "info"; icon: string; text: string } {
  if (b.decision && !b.decision.allowed) return { tone: "danger", icon: icons.handStop, text: "断った" };
  if (b.ok === undefined) return { tone: "info", icon: icons.refresh, text: b.tool ? "動いている" : "書いている" };
  return b.ok ? { tone: "ok", icon: icons.check, text: "済んだ" } : { tone: "danger", icon: icons.alertTriangle, text: "失敗" };
}

function Tool(props: { b: Of<"tool">; now: number }) {
  const kind = () => KINDS[props.b.toolKind];
  const checks = () => checksOf(props.b.toolKind, props.b.output ?? props.b.working);
  const took = () => {
    if (props.b.at === 0n) return "";
    const end = props.b.endedAt ? Number(props.b.endedAt) : props.now;
    return durationText(end - Number(props.b.at));
  };
  const summary = () => summaryOf(props.b.tool, props.b.input) || props.b.tool;
  const bad = () => props.b.ok === false || (props.b.decision !== undefined && !props.b.decision.allowed) || (checks()?.failed ?? 0) > 0;
  const command = () => {
    const input = props.b.input;
    return typeof input === "object" && input !== null && "command" in input ? String(Reflect.get(input, "command")) : "";
  };
  // What went wrong opens by itself, also when it is known only after it is drawn.
  const [open, setOpen] = createSignal(bad() && props.b.toolKind !== "read");
  createEffect(on(bad, (b) => b && props.b.toolKind !== "read" && setOpen(true), { defer: true }));
  const title: JSX.Element = (
    <span class={`run-tool-head ${props.b.toolKind}`}>
      <Icon svg={kind().icon} size={15} />
      <span class="run-tool-kind">{kind().text}</span>
      <span class="run-tool-what mono">{summary()}</span>
      <Show when={checks()}>
        {(c) => (
          <Tag tone={c().failed > 0 ? "danger" : "ok"} icon={c().failed > 0 ? icons.circleX : icons.circleCheck}>
            {c().passed} 通過・{c().failed} 失敗
          </Tag>
        )}
      </Show>
      <Show when={props.b.changes.length > 0}>
        <span class="run-diff-count">
          <span class="run-diff-add">+{props.b.changes.reduce((n, c) => n + (c.diff.match(/^\+/gm)?.length ?? 0), 0)}</span>
          <span class="run-diff-del">−{props.b.changes.reduce((n, c) => n + (c.diff.match(/^-/gm)?.length ?? 0), 0)}</span>
        </span>
      </Show>
      <Tag tone={standing(props.b).tone} icon={standing(props.b).icon}>
        {standing(props.b).text}
      </Tag>
      <Show when={took()}>
        <span class="run-took">{took()}</span>
      </Show>
    </span>
  );
  return (
    <Disclosure class={`run-tool${bad() ? " bad" : ""}`} title={title} open={open()} onOpenChange={setOpen}>
      <div class="run-tool-body">
        <Switch
          fallback={
            <Show when={props.b.input !== undefined || props.b.inputText}>
              <pre class="run-pre">{props.b.inputText || (typeof props.b.input === "string" ? props.b.input : JSON.stringify(props.b.input, null, 2))}</pre>
            </Show>
          }
        >
          <Match when={command()}>
            <pre class="run-pre run-command">
              <span class="run-prompt" aria-hidden="true">
                ${" "}
              </span>
              {command()}
            </pre>
          </Match>
          <Match when={props.b.changes.length > 0}>
            {props.b.changes.map((c) => (
              <RunDiff path={c.path} diff={c.diff} />
            ))}
          </Match>
        </Switch>
        <Show when={props.b.decision && !props.b.decision.allowed}>
          <p class="run-refused">
            <Icon svg={icons.handStop} size={14} /> 使わせなかった: {props.b.decision?.why}
          </p>
        </Show>
        <Show when={props.b.output ?? props.b.working}>
          {(out) => (
            <pre class={`run-pre run-out${props.b.ok === false ? " bad" : ""}`} aria-label="出力">
              {tail(out())}
            </pre>
          )}
        </Show>
        <Show when={props.b.cut}>
          <p class="sub">長いので、残したのは一部だけ。</p>
        </Show>
      </div>
    </Disclosure>
  );
}

function Question(props: { b: Of<"question">; run: WorkerRun }) {
  // Whatever was told of it before: a question taken back may be asked again.
  const waits = () => props.run.state === "waiting" && props.run.question?.id === props.b.id;
  return (
    <div class={`run-question${waits() ? " waits" : ""}`}>
      <span class="run-mark" aria-hidden="true">
        <Icon svg={icons.handStop} size={15} />
      </span>
      <div class="grow">
        <div class="run-meta">
          <b>{waits() ? "あなたの答えを待っている" : "尋ねた"}</b>
          <Show when={props.b.tool}>
            <span class="mono">{props.b.tool}</span>
          </Show>
          <At at={props.b.at} />
        </div>
        <p class="run-question-what">{props.b.what}</p>
        <Show when={waits()} fallback={<Answered b={props.b} />}>
          <RunAsk run={props.run} />
        </Show>
      </div>
    </div>
  );
}

function Answered(props: { b: Of<"question"> }) {
  return (
    <Switch>
      <Match when={props.b.decision}>
        {(d) => (
          <Tag tone={d().allowed ? "ok" : "danger"} icon={d().allowed ? icons.check : icons.handStop}>
            {d().allowed ? (props.b.tool === "AskUserQuestion" ? "答えた" : "許した") : "断った"}
            {d().why ? `: ${d().why}` : ""}
          </Tag>
        )}
      </Match>
      <Match when={props.b.withdrawn}>
        <Tag>待たれなくなった（ほかで答えたか、取り下げられた）</Tag>
      </Match>
    </Switch>
  );
}

/** One thing a run did, as its stream shows it. */
export default function RunBlock(props: { b: Block; run: WorkerRun; now: number }) {
  return (
    <Switch>
      <Match when={props.b.kind === "said" && props.b}>{(b) => <Said b={b()} />}</Match>
      <Match when={props.b.kind === "tool" && props.b}>{(b) => <Tool b={b()} now={props.now} />}</Match>
      <Match when={props.b.kind === "question" && props.b}>{(b) => <Question b={b()} run={props.run} />}</Match>
      <Match when={props.b.kind === "thought" && props.b}>
        {(b) => (
          <Disclosure class="run-thought" title={<span class="run-tool-head"><Icon svg={icons.brain} size={15} /> 考えたこと{b().live ? "（考えている）" : ""}</span>}>
            <p class="run-thought-text">{b().text}</p>
          </Disclosure>
        )}
      </Match>
      <Match when={props.b.kind === "asked" && props.b}>
        {(b) => (
          <Disclosure class="run-asked" title={<span class="run-tool-head"><Icon svg={icons.send} size={15} /> 頼み<At at={b().at} /></span>}>
            <Markdown text={b().text} />
          </Disclosure>
        )}
      </Match>
      <Match when={props.b.kind === "plan" && props.b}>
        {(b) => (
          <p class="run-note plan">
            <Icon svg={icons.listCheck} size={14} /> 段取りを書き直した（{b().steps.length} 段）
            <At at={b().at} />
          </p>
        )}
      </Match>
      <Match when={props.b.kind === "note" && props.b}>
        {(b) => (
          <p class={`run-note ${b().tone}`}>
            <Icon svg={b().tone === "ok" ? icons.circleCheck : b().tone === "danger" ? icons.alertTriangle : icons.flag} size={14} /> {b().text}
            <At at={b().at} />
          </p>
        )}
      </Match>
    </Switch>
  );
}
