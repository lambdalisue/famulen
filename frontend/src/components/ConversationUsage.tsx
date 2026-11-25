import { createAsync } from "@solidjs/router";
import { createMemo, For, Show } from "solid-js";
import type { UsageSpan } from "~/gen/famulen/v1/settings_pb";
import { conversationUsageQuery, shortTokens } from "~/lib/models";

/** What conversations that the owner did not begin are of. */
const SOURCES: Record<string, string> = {
  slack: "Slack",
  schedule: "定期の仕事",
  wake: "起床予約",
  arrival: "つなぎ先の知らせ",
  lookout: "見張り",
  approval: "確認の答え",
  worker: "委託の終わり",
};

interface Spent {
  turns: bigint;
  inputTokens: bigint;
  cachedTokens: bigint;
  outputTokens: bigint;
}

function Cells(props: { spent: Spent | undefined }) {
  const turns = () => Number(props.spent?.turns ?? 0n);
  const input = () => Number(props.spent?.inputTokens ?? 0n);
  return (
    <>
      <td class="mono num">{turns()} 回</td>
      <td class="mono num">{shortTokens(turns() > 0 ? Math.round(input() / turns()) : 0)}</td>
      <td class="mono num">{shortTokens(input())}</td>
      <td class="mono num">{shortTokens(Number(props.spent?.cachedTokens ?? 0n))}</td>
      <td class="mono num">{shortTokens(Number(props.spent?.outputTokens ?? 0n))}</td>
    </>
  );
}

/** What each of the secretary's conversations used in a period. */
export default function ConversationUsage(props: { span: UsageSpan; day: string }) {
  const usage = createAsync(() => conversationUsageQuery(props.span, props.day));
  const rows = createMemo(() => usage()?.conversations ?? []);
  return (
    <Show when={rows().length > 0}>
      <section class="usage-detail">
        <h2>秘書の会話ごと</h2>
        <div class="scroll">
          <table class="table" aria-label="秘書の会話ごとの使った量">
            <thead>
              <tr>
                <th>会話</th>
                <th class="num">ターン</th>
                <th class="num">1 回あたりの入力</th>
                <th class="num">入力</th>
                <th class="num">うちキャッシュ</th>
                <th class="num">出力</th>
              </tr>
            </thead>
            <tbody>
              <tr class="usage-group">
                <td>すべての会話</td>
                <Cells spent={usage()?.total} />
              </tr>
              <For each={rows()}>
                {(c) => (
                  <tr class="usage-part">
                    <td>
                      {SOURCES[c.source] ? `${SOURCES[c.source]} · ` : ""}
                      {c.title}
                    </td>
                    <Cells spent={c.spent} />
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </div>
      </section>
    </Show>
  );
}
