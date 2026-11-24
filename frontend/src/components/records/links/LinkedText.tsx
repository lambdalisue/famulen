import { A } from "@solidjs/router";
import { For, Show } from "solid-js";
import { hrefOf, linkOf } from "~/lib/records";
import { memoryHref } from "~/lib/memory";
import { useKnownLink } from "./known";

const LINK = /^\[\[[^[\]\n]+\]\]$/;

/** A wiki link, or code, whose links are only shown. */
const PART = /(?<![`\\])(`+)(?!`).*?(?<!`)\1(?!`)|\[\[[^[\]\n]+\]\]/gs;

function split(text: string): string[] {
  const parts: string[] = [];
  let at = 0;
  for (const m of text.matchAll(PART)) {
    parts.push(text.slice(at, m.index), m[0]);
    at = m.index + m[0].length;
  }
  return [...parts, text.slice(at)];
}

function Link(props: { text: string }) {
  const known = useKnownLink();
  const inner = () => props.text.slice(2, -2).split("|");
  const record = () => linkOf(props.text);
  const now = () => {
    const r = record();
    return r && known(r.kind, r.id);
  };
  const title = () => inner()[1]?.trim() || now()?.title || inner()[0];
  const href = () => {
    const r = record();
    return r ? hrefOf(r.kind, r.id) : memoryHref(inner()[0]);
  };
  return (
    <Show when={!now()?.missing} fallback={<span class="md-link-missing">{title()}（もうない）</span>}>
      <Show when={href()} fallback={title()}>
        {(to) => (
          <A href={to()} title={now()?.title}>
            {title()}
          </A>
        )}
      </Show>
    </Show>
  );
}

/** A line of plain text whose wiki links can be followed. */
export default function LinkedText(props: { text: string }) {
  return <For each={split(props.text)}>{(part) => (LINK.test(part) ? <Link text={part} /> : part)}</For>;
}
