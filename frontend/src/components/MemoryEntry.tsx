import { createAsync, useNavigate } from "@solidjs/router";
import { Show } from "solid-js";
import AskAbout from "~/components/AskAbout";
import Markdown from "~/components/Markdown";
import type { MemoryItem } from "~/gen/famulen/v1/memory_pb";
import { LinkButton, ListRow, Tag } from "~/components/ui";
import { workgroupsQuery } from "~/lib/data";
import * as icons from "~/lib/icons";
import { itemHref } from "~/lib/memory";
import { workgroupNames } from "~/lib/records";

export function SrcTag(props: { src: string }) {
  return (
    <Show when={props.src === "said" || props.src === "confirmed"} fallback={<Tag tone="guess">推し量り</Tag>}>
      <Show when={props.src === "said"} fallback={<Tag tone="ok">確かめた</Tag>}>
        <Tag tone="said">あなたが言った</Tag>
      </Show>
    </Show>
  );
}

/** From when until when a memory holds; empty when it says neither. */
export function spanOf(attrs: MemoryItem["attrs"]): string {
  if (attrs.from && attrs.to) return `${attrs.from} から ${attrs.to} まで`;
  if (attrs.from) return `${attrs.from} から今も`;
  return attrs.to ? `${attrs.to} まで` : "";
}

export const workgroupIds = (attrs: MemoryItem["attrs"]) => (attrs.workgroup ?? "").split(",").filter(Boolean);

/** The way to an item's details from the keyboard; its whole row takes presses. */
export function DetailLink(props: { item: MemoryItem }) {
  return (
    <LinkButton
      small
      variant="ghost"
      icon={icons.listDetails}
      label={`${props.item.text} ― 詳細を見る`}
      href={itemHref(props.item)}
      noScroll
    />
  );
}

/** Open the details of the item picked; an item's address is kept, so the way back closes it. */
export function usePick() {
  const navigate = useNavigate();
  return (item: MemoryItem) => navigate(itemHref(item), { scroll: false });
}

/** One remembered item with where it came from, its span, evidence and use. */
export function EntryRow(props: { item: MemoryItem }) {
  const pick = usePick();
  const workgroups = createAsync(() => workgroupsQuery());
  const meta = () => {
    const a = props.item.attrs;
    return [spanOf(a), a.ev && `根拠 ${a.ev}`, a.uses && `使った ${a.uses} 回`, workgroupNames(workgroups(), workgroupIds(a))]
      .filter(Boolean)
      .join(" · ");
  };
  return (
    <ListRow
      class="memory-row"
      lead={<SrcTag src={props.item.src} />}
      title={<Markdown inline text={props.item.text} />}
      sub={meta() || undefined}
      onPick={() => pick(props.item)}
      end={
        <>
          <AskAbout what={`記憶：${props.item.text}`} />
          <DetailLink item={props.item} />
        </>
      }
    />
  );
}
