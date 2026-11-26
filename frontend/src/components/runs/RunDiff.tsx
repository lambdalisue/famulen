import { For } from "solid-js";

/** How many lines of one change are drawn before the rest is left out. */
const MOST_LINES = 400;

type Line = { kind: "hunk" | "add" | "del" | "same"; text: string };

function lines(diff: string): Line[] {
  return diff
    .replace(/\n$/, "")
    .split("\n")
    .map((text) => {
      if (text.startsWith("@@")) return { kind: "hunk", text };
      if (text.startsWith("+")) return { kind: "add", text: text.slice(1) };
      if (text.startsWith("-")) return { kind: "del", text: text.slice(1) };
      return { kind: "same", text: text.slice(1) };
    });
}

const SIGNS = { hunk: "", add: "+", del: "−", same: " " };

/** A change of one file as a unified diff, its lines marked by sign and color. */
export default function RunDiff(props: { path: string; diff: string }) {
  const all = () => lines(props.diff);
  const counts = () => ({ add: all().filter((l) => l.kind === "add").length, del: all().filter((l) => l.kind === "del").length });
  return (
    <figure class="run-diff">
      <figcaption>
        <span class="mono grow">{props.path}</span>
        <span class="run-diff-add">+{counts().add}</span>
        <span class="run-diff-del">−{counts().del}</span>
      </figcaption>
      <pre aria-label={`${props.path} の差分`}>
        <For each={all().slice(0, MOST_LINES)}>
          {(l) => (
            <span class={`run-diff-line ${l.kind}`}>
              <span class="run-diff-sign" aria-hidden="true">
                {SIGNS[l.kind]}
              </span>
              {l.text}
              {"\n"}
            </span>
          )}
        </For>
      </pre>
      {all().length > MOST_LINES ? <p class="sub">ほか {all().length - MOST_LINES} 行</p> : null}
    </figure>
  );
}
