import { createAsync, useSearchParams } from "@solidjs/router";
import { createEffect, For } from "solid-js";
import RecordDetail from "~/components/records/RecordDetail";
import RecordsPage, { oneParam } from "~/components/records/RecordsPage";
import { workgroupsQuery } from "~/lib/data";
import { plainText, type Rec, recordQuery, workgroupNames } from "~/lib/records";
import { setView } from "~/lib/view";

function excerpt(r: Rec): string {
  return plainText(r.notes)
    .replace(/[#>*`|_-]+/g, " ")
    .replace(/\s+/g, " ")
    .slice(0, 160);
}

export default function Memos() {
  const [params] = useSearchParams();
  const workgroups = createAsync(() => workgroupsQuery());
  const picked = createAsync(() => {
    const id = oneParam(params.id);
    return id ? recordQuery("memos", id) : Promise.resolve(undefined);
  });
  createEffect(() => setView(picked() ? `メモ · ${picked()?.title}` : "メモ"));
  const body = (r: Rec) => (
    <span class="rec-main">
      <b>{r.title}</b>
      <span class="rec-excerpt">{excerpt(r)}</span>
      <span class="rec-sub">
        <span class="mono">{r.updated.slice(0, 10)}</span>
        <For each={r.labels}>{(l) => <span class="tag label">{l}</span>}</For>
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
