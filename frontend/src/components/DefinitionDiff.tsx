import { Collapsible } from "@ark-ui/solid/collapsible";
import { For } from "solid-js";
import { type DiffLine, diffLines, type Fold, folded } from "~/lib/diff";

const MARK: Record<DiffLine["kind"], string> = { same: " ", del: "-", add: "+" };

function Line(props: { line: DiffLine }) {
  return (
    <div class={`diff-line ${props.line.kind}`} data-kind={props.line.kind}>
      <span class="diff-mark" aria-hidden="true">
        {MARK[props.line.kind]}
      </span>
      {props.line.text}
    </div>
  );
}

function Folded(props: { fold: Fold }) {
  return (
    <Collapsible.Root class="diff-fold">
      <Collapsible.Trigger class="diff-fold-trigger">… {props.fold.lines.length} 行おなじ</Collapsible.Trigger>
      <Collapsible.Content>
        <For each={props.fold.lines}>{(line) => <Line line={line} />}</For>
      </Collapsible.Content>
    </Collapsible.Root>
  );
}

/**
 * How a service definition's new text differs from the one in use, line by
 * line. Unchanged stretches are folded and open on a press.
 */
export default function DefinitionDiff(props: { before: string; after: string }) {
  const shown = () => folded(diffLines(props.before, props.after));
  const counts = () => {
    const all = diffLines(props.before, props.after);
    return {
      add: all.filter((l) => l.kind === "add").length,
      del: all.filter((l) => l.kind === "del").length,
    };
  };
  return (
    <div class="diff" data-testid="definition-diff">
      <div class="diff-head sub">
        <span class="diff-count add">+{counts().add} 行</span>
        <span class="diff-count del">−{counts().del} 行</span>
        {props.before === "" ? <span>新しい定義</span> : null}
      </div>
      <div class="diff-body mono">
        <For each={shown()}>{(part) => (part.kind === "fold" ? <Folded fold={part} /> : <Line line={part} />)}</For>
      </div>
    </div>
  );
}
