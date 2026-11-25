import { Show } from "solid-js";
import AskAbout from "~/components/AskAbout";
import Markdown from "~/components/Markdown";
import type { MemoryItem } from "~/gen/famulen/v1/memory_pb";
import { ListRow, Tag } from "~/components/ui";

export function SrcTag(props: { src: string }) {
  return (
    <Show when={props.src === "said" || props.src === "confirmed"} fallback={<Tag tone="guess">推し量り</Tag>}>
      <Show when={props.src === "said"} fallback={<Tag tone="ok">確かめた</Tag>}>
        <Tag tone="said">あなたが言った</Tag>
      </Show>
    </Show>
  );
}

/** One remembered item with where it came from, its span, evidence and use. */
export function EntryRow(props: { item: MemoryItem }) {
  const meta = () => {
    const a = props.item.attrs;
    return [a.from && `${a.from} から`, a.to && `${a.to} まで`, a.ev && `根拠 ${a.ev}`, a.uses && `使った ${a.uses} 回`].filter(
      (m): m is string => !!m,
    );
  };
  return (
    <ListRow
      class="memory-row"
      lead={<SrcTag src={props.item.src} />}
      title={<Markdown inline text={props.item.text} />}
      sub={meta().length > 0 ? meta().join(" · ") : undefined}
      end={<AskAbout what={`記憶：${props.item.text}`} />}
    />
  );
}
