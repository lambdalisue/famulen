import RecordDetail from "~/components/records/detail/RecordDetail";
import RecordsPage from "~/components/records/RecordsPage";
import { dateText } from "~/lib/dates";
import { usePicked } from "~/lib/picked";
import { excerptOf, type Rec } from "~/lib/records";

const newest = (records: Rec[]) => [{ name: "", records: [...records].sort((a, b) => b.updated.localeCompare(a.updated)) }];

export default function Memos() {
  usePicked({ kind: () => "memos", section: () => "メモ" });
  return (
    <RecordsPage
      kind="memos"
      base="/memos"
      title="メモ"
      narrow={[]}
      over={(r) => !!r.movedTo}
      overLabel="タスクに移したメモも"
      groups={newest}
      row={(r) => ({ sub: () => <span>{dateText(r().updated.slice(0, 10))}</span>, excerpt: () => excerptOf(r().notes) })}
      cards
      detail={(id) => <RecordDetail kind="memos" id={id} back="/memos" />}
      add={{ label: "メモを書く", titleLabel: "題名", fields: [] }}
      empty="メモはまだない"
    />
  );
}
