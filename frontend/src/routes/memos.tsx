import { createAsync } from "@solidjs/router";
import { createEffect, For } from "solid-js";
import RecordDetail from "~/components/records/RecordDetail";
import RecordsPage from "~/components/records/RecordsPage";
import { workgroupsQuery } from "~/lib/data";
import { excerptOf, type Rec, workgroupNames } from "~/lib/records";
import { dateText } from "~/lib/dates";
import { usePicked } from "~/lib/picked";
import { Tag } from "~/components/ui";

export default function Memos() {
  const workgroups = createAsync(() => workgroupsQuery());
  usePicked({ kind: () => "memos", section: () => "メモ" });
  const body = (r: Rec) => (
    <span class="rec-main">
      <b>{r.title}</b>
      <span class="rec-excerpt">{excerptOf(r.notes)}</span>
      <span class="rec-sub">
        <span>{dateText(r.updated.slice(0, 10))}</span>
        <For each={r.labels}>{(l) => <Tag tone="label">{l}</Tag>}</For>
        <span>{workgroupNames(workgroups(), r.workgroups)}</span>
      </span>
    </span>
  );
  return (
    <RecordsPage
      kind="memos"
      base="/memos"
      title="メモ"
      narrow={[]}
      over={(r) => !!r.movedTo}
      overLabel="タスクに移したメモも"
      groups={(records) => [{ name: "", records: [...records].sort((a, b) => b.updated.localeCompare(a.updated)) }]}
      row={body}
      card={body}
      detail={(id) => <RecordDetail kind="memos" id={id} back="/memos" />}
      add={{ label: "メモを書く", titleLabel: "題名", fields: [] }}
      empty="メモはまだない。"
    />
  );
}
