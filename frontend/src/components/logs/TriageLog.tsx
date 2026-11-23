import { shortTime } from "~/lib/dates";
import { createAsync, query } from "@solidjs/router";
import { For, Show } from "solid-js";
import { api } from "~/lib/api";
import { useOwner } from "~/lib/owner";
import { createPages, PAGE, PageNav } from "./Pages";

const triageLogQuery = query(async (before: number) => {
  const page = await api.log.listTriageLog({ before: BigInt(before), limit: PAGE });
  return { rows: page.decisions, more: page.more };
}, "triageLog");

/** What triage decided of each message in an attended channel. */
export default function TriageLog() {
  const pages = createPages(triageLogQuery);
  const owner = useOwner();
  return (
    <>
      <p class="sub">注視しているチャンネルの発言ごとに、秘書宛てと見て秘書に渡したか、見送ったか。発言は最初の行だけが残る。</p>
      <div class="list">
        <For each={pages.rows()}>
          {(d) => (
            <div class="item triage-log-entry">
              <span class="mono sub" style={{ width: "90px" }}>
                {shortTime(d.at, owner.tz())}
              </span>
              <span class={`tag${d.addressed ? " aide" : ""}`}>{d.addressed ? "秘書へ" : "見送った"}</span>
              <div class="grow">
                <div style={{ "font-size": "13px", "overflow-wrap": "anywhere" }}>{d.title}</div>
                <div class="sub">
                  {d.source} · {d.reason}
                </div>
              </div>
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
