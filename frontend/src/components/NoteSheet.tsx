import { Dialog } from "@ark-ui/solid/dialog";
import { createMemo, Show } from "solid-js";
import Markdown from "~/components/Markdown";
import { SrcTag } from "~/components/MemoryEntry";
import { Facts, Origin } from "~/components/MemoryFacts";
import { SideSheet } from "~/components/ui";
import type { MemoryItem } from "~/gen/famulen/v1/memory_pb";
import { dateText } from "~/lib/dates";

const JUDGED: Record<string, string> = {
  said: "確かな事実（あなたが言った）",
  confirmed: "確かな事実（あなたが確かめた）",
};

const STATES: Record<string, string> = { waiting: "夢待ち", record: "記録へ移すのを待つ" };

/** What a note says of itself, and where it came from. */
export default function NoteSheet(props: { note: MemoryItem | undefined; onClose: () => void }) {
  // What it showed stays while it closes.
  const shown = createMemo<MemoryItem | undefined>((before) => props.note ?? before);
  const day = (e: MemoryItem) => e.file.replace(/^.*\/|\.md$/g, "");
  return (
    <Dialog.Root open={!!props.note} onOpenChange={(d) => !d.open && props.onClose()} lazyMount unmountOnExit>
      <SideSheet name="note" title="気づきの詳細">
        <Show when={shown()} keyed>
          {(e) => (
            <div class="stack">
              <p>
                <Markdown inline text={e.text} />
              </p>
              <SrcTag src={e.src} />
              <Facts
                rows={[
                  ["記録した時刻", [dateText(day(e)), e.time].filter(Boolean).join(" ")],
                  ["書いたもの", e.attrs.by === "lookout" ? "見張り" : ""],
                  ["確かさ", JUDGED[e.src] ?? "推し量り"],
                  ["状態", STATES[e.attrs.state ?? ""] ?? "夢待ち"],
                  ["答えた判断", e.attrs.judges && <a href={`/memory/pending?item=${encodeURIComponent(e.attrs.judges)}`}>保留中の判断を開く</a>],
                ]}
              />
              <h3>出どころ</h3>
              <Origin id={e.id} />
            </div>
          )}
        </Show>
      </SideSheet>
    </Dialog.Root>
  );
}
