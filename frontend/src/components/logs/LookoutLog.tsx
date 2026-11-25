import { shortTime } from "~/lib/dates";
import { createAsync, query } from "@solidjs/router";
import { For, Show } from "solid-js";
import AskAbout from "~/components/AskAbout";
import { type LookoutDecision, LookoutOutcome } from "~/gen/famulen/v1/lookout_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import * as icons from "~/lib/icons";
import { useOwner } from "~/lib/owner";
import { createPages, PAGE, PageNav } from "./Pages";
import { EmptyState, IconButton, List, Tag } from "~/components/ui";

const lookoutLogQuery = query(async (before: number) => {
  const page = await api.log.listLookoutLog({ before: BigInt(before), limit: PAGE });
  return { rows: page.decisions, more: page.more };
}, "lookoutLog");

/** Whether the lookout passed the item on, so the owner's answer would be "not wanted". */
function passedOn(d: LookoutDecision): boolean {
  return d.passedOn || d.outcomeKind === LookoutOutcome.DIGEST;
}

/** What the lookout decided about each item it read, and the owner's answers to it. */
export default function LookoutLog() {
  const pages = createPages(lookoutLogQuery);
  const owner = useOwner();
  return (
    <>
      <p class="sub">
        見張りが読んだものと、それぞれを関わりがないとしたか・朝のまとめに入れたか・秘書へ渡したか、そのわけ。判断が違えば、親指で教える。教えたことは記憶に入り、夢のあとの判断から効く。
      </p>
      <List>
        <For each={pages.rows()}>
          {(d) => (
            <div class="item wraps-phone" role="listitem">
              <span class="mono sub when">{shortTime(d.at, owner.tz())}</span>
              <Tag tone={d.passedOn ? "aide" : undefined}>{d.outcome}</Tag>
              <div class="grow">
                <div class="item-title">{d.title}</div>
                <Show when={d.reason}>
                  <div class="sub">{d.reason}</div>
                </Show>
              </div>
              <AskAbout framed what={`見張りの判断「${d.title}」（${d.outcome}）`} />
              <Show when={d.feedback === 0} fallback={<span class="sub">{d.feedback > 0 ? "拾ってほしかった" : "要らなかった"}</span>}>
                <Show
                  when={passedOn(d)}
                  fallback={
                    <IconButton
                      icon={icons.thumbUp}
                      label="拾ってほしかった"
                      onClick={() => act(() => api.lookout.giveFeedback({ id: d.id, wanted: true }), "覚えた")}
                    />
                  }
                >
                  <IconButton
                    icon={icons.thumbDown}
                    label="要らなかった"
                    onClick={() => act(() => api.lookout.giveFeedback({ id: d.id, wanted: false }), "覚えた")}
                  />
                </Show>
              </Show>
            </div>
          )}
        </For>
        <Show when={pages.loaded() && pages.rows().length === 0}>
          <EmptyState>まだない</EmptyState>
        </Show>
      </List>
      <PageNav pages={pages} />
    </>
  );
}
