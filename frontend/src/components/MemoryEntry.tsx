import { Show } from "solid-js";
import type { MemoryItem } from "~/gen/famulen/v1/memory_pb";
import { setSelection } from "~/lib/view";

export function SrcTag(props: { src: string }) {
  const tag = () =>
    props.src === "said"
      ? ["tag said", "あなたが言った"]
      : props.src === "confirmed"
        ? ["tag confirmed", "確かめた"]
        : ["tag inferred", "推し量り"];
  return <span class={tag()[0]}>{tag()[1]}</span>;
}

/** One remembered item with where it came from, its span, evidence and use. */
export function EntryRow(props: { item: MemoryItem }) {
  const meta = () => {
    const a = props.item.attrs;
    return [
      a.from && `${a.from} から`,
      a.to && `${a.to} まで`,
      a.ev && `根拠 ${a.ev}`,
      a.uses && `使った ${a.uses} 回`,
    ].filter((m): m is string => !!m);
  };
  return (
    <div class="item">
      <SrcTag src={props.item.src} />
      <div class="grow" style={{ cursor: "pointer" }} onClick={() => setSelection(`記憶：${props.item.text}`)}>
        <div style={{ "font-size": "14px", "line-height": 1.6 }}>{props.item.text}</div>
        <Show when={meta().length > 0}>
          <div class="sub">{meta().join(" · ")}</div>
        </Show>
      </div>
    </div>
  );
}
