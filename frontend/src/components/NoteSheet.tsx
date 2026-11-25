import { Dialog } from "@ark-ui/solid/dialog";
import { create } from "@bufbuild/protobuf";
import { createAsync } from "@solidjs/router";
import { createMemo, ErrorBoundary, For, type JSX, Show, Suspense } from "solid-js";
import Markdown from "~/components/Markdown";
import { SrcTag } from "~/components/MemoryEntry";
import { EmptyState, ErrorState, LinkButton, Loading, SideSheet } from "~/components/ui";
import { type MemoryItem, NoteOriginSchema } from "~/gen/famulen/v1/memory_pb";
import { dateText, dateTimeText } from "~/lib/dates";
import * as icons from "~/lib/icons";
import { noteOriginQuery } from "~/lib/memory";
import { useOwner } from "~/lib/owner";

const JUDGED: Record<string, string> = {
  said: "確かな事実（あなたが言った）",
  confirmed: "確かな事実（あなたが確かめた）",
};

const STATES: Record<string, string> = { waiting: "夢待ち", record: "記録へ移すのを待つ" };

const KINDS: Record<string, string> = { mail: "メール", slack: "Slack" };

/** The values that are known, each under what it is. */
function Facts(props: { rows: [label: string, value: JSX.Element][] }) {
  return (
    <dl class="memory-working">
      <For each={props.rows.filter(([, value]) => !!value)}>
        {([label, value]) => (
          <>
            <dt>{label}</dt>
            <dd>{value}</dd>
          </>
        )}
      </For>
    </dl>
  );
}

function Origin(props: { id: string }) {
  const owner = useOwner();
  const origin = createAsync(async () => (props.id ? noteOriginQuery(props.id) : create(NoteOriginSchema)));
  return (
    <Show when={origin()} fallback={<Loading />}>
      {(o) => (
        <Show when={o().kind} fallback={<EmptyState>出どころは残っていない</EmptyState>}>
          <Facts
            rows={[
              ["どこ", [KINDS[o().kind] ?? o().kind, o().account].filter(Boolean).join(" · ")],
              ["チャンネル", o().channel],
              [o().kind === "mail" ? "差出人" : "書いた人", o().from],
              ["件名", o().subject],
              ["日時", o().at ? dateTimeText(o().at, owner.tz()) : ""],
              ["抜粋", o().excerpt],
              ["見張りの判断", [o().outcome, o().reason].filter((t, i, all) => t && all.indexOf(t) === i).join(" · ")],
            ]}
          />
          <Show when={o().url}>
            {(url) => (
              <LinkButton small icon={icons.externalLink} href={url()}>
                {o().kind === "mail" ? "メールを開く" : "発言を開く"}
              </LinkButton>
            )}
          </Show>
        </Show>
      )}
    </Show>
  );
}

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
                ]}
              />
              <h3>出どころ</h3>
              <ErrorBoundary fallback={(error) => <ErrorState error={error} />}>
                <Suspense fallback={<Loading />}>
                  <Origin id={e.id} />
                </Suspense>
              </ErrorBoundary>
            </div>
          )}
        </Show>
      </SideSheet>
    </Dialog.Root>
  );
}
