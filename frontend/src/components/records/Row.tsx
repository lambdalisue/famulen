import { createAsync } from "@solidjs/router";
import { For, type JSX, Show } from "solid-js";
import { ListRow, Tag } from "~/components/ui";
import { workgroupsQuery } from "~/lib/data";
import { type Rec, workgroupNames } from "~/lib/records";

/** How one record reads in a list, beyond its title. */
export type RowParts = {
  lead?: () => JSX.Element;
  title?: () => JSX.Element;
  sub?: () => JSX.Element;
  end?: () => JSX.Element;
  /** The start of what is written in it. */
  excerpt?: () => string;
};

/** What every kind ends its line with: its labels and workgroups. */
export function RowTags(props: { record: Rec }) {
  const workgroups = createAsync(() => workgroupsQuery());
  return (
    <>
      <For each={props.record.labels}>{(l) => <Tag tone="label">{l}</Tag>}</For>
      <Show when={workgroupNames(workgroups(), props.record.workgroups)}>{(names) => <span>{names()}</span>}</Show>
    </>
  );
}

/** One record in a list, which opens it. */
export function RecordRow(props: { record: Rec; href: string; current?: boolean; over?: boolean; card?: boolean; parts: RowParts }) {
  return (
    <ListRow
      class="rec-row"
      classList={{ "rec-card": props.card, over: props.over, doubted: !!props.record.obsolete }}
      record={props.record.id}
      href={props.href}
      current={props.current}
      lead={props.parts.lead?.()}
      title={props.parts.title?.() ?? props.record.title}
      sub={
        <>
          <Show when={props.record.obsolete}>
            <Tag tone="warn" title={props.record.obsoleteReason}>
              古いかも
            </Tag>
          </Show>
          {props.parts.sub?.()}
          <RowTags record={props.record} />
          <Show when={props.parts.excerpt?.()}>{(text) => <span class="rec-excerpt">{text()}</span>}</Show>
        </>
      }
      end={props.parts.end?.()}
    />
  );
}
