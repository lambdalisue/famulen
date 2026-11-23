import { createAsync, query } from "@solidjs/router";
import { For, Show } from "solid-js";
import Icon from "~/components/Icon";
import { type LookoutDecision, LookoutOutcome } from "~/gen/famulen/v1/lookout_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { personaQuery } from "~/lib/data";
import * as icons from "~/lib/icons";
import { shortTime } from "~/lib/time";
import { setSelection } from "~/lib/view";
import { createPages, PAGE, PageNav } from "./Pages";

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
  const persona = createAsync(() => personaQuery());
  return (
    <>
      <p class="sub">
        見張りが読んだものと、それぞれを関わりがないとしたか・朝のまとめに入れたか・秘書へ渡したか、そのわけ。判断が違えば、親指で教える。教えたことは記憶に入り、夢のあとの判断から効く。
      </p>
        <div class="list">
          <For each={pages.rows()}>
            {(d) => (
              <div class="item" onClick={() => setSelection(`見張りの判断「${d.title}」（${d.outcome}）`)}>
                <span class="mono sub" style={{ width: "90px" }}>
                  {shortTime(d.at, persona()?.timezone ?? "")}
                </span>
                <span class={`tag${d.passedOn ? " aide" : ""}`}>{d.outcome}</span>
                <div class="grow">
                  <div style={{ "font-size": "13px" }}>{d.title}</div>
                  <Show when={d.reason}>
                    <div class="sub">{d.reason}</div>
                  </Show>
                </div>
                <Show
                  when={d.feedback === 0}
                  fallback={<span class="sub">{d.feedback > 0 ? "拾ってほしかった" : "要らなかった"}</span>}
                >
                  <Show
                    when={passedOn(d)}
                    fallback={
                      <button
                        type="button"
                        class="btn small"
                        aria-label="拾ってほしかった"
                        onClick={() => act(() => api.lookout.giveFeedback({ id: d.id, wanted: true }), "覚えた")}
                      >
                        <Icon svg={icons.thumbUp} size={12} />
                      </button>
                    }
                  >
                    <button
                      type="button"
                      class="btn small"
                      aria-label="要らなかった"
                      onClick={() => act(() => api.lookout.giveFeedback({ id: d.id, wanted: false }), "覚えた")}
                    >
                      <Icon svg={icons.thumbDown} size={12} />
                    </button>
                  </Show>
                </Show>
              </div>
            )}
          </For>
          <Show when={pages.loaded() && pages.rows().length === 0}>
            <p class="empty">まだない</p>
          </Show>
        </div>
        <PageNav pages={pages} />
    </>
  );
}
