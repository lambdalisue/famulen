import { A, createAsync, query } from "@solidjs/router";
import { For, Show } from "solid-js";
import type { Dream } from "~/gen/famulen/v1/memory_pb";
import { api } from "~/lib/api";
import { personaQuery } from "~/lib/data";
import { nightLabel } from "~/lib/memory";
import { createPages, PAGE, PageNav } from "./Pages";

const dreamLogQuery = query(async (before: number) => {
  const page = await api.log.listDreamLog({ before: BigInt(before), limit: PAGE });
  return { rows: page.dreams, more: page.more };
}, "dreamLog");

function counts(d: Dream): string {
  const parts: [string, number][] = [
    ["足した", d.added],
    ["直した", d.edited],
    ["置き換えた", d.replaced],
    ["忘れた", d.archived],
    ["確かめた", d.confirmed],
    ["保留にした", d.pendingAdded],
  ];
  return parts
    .filter(([, n]) => n > 0)
    .map(([label, n]) => `${label} ${n}`)
    .join(" · ");
}

/** Each night the dream ran: how it went and how much it changed. */
export default function DreamLog() {
  const pages = createPages(dreamLogQuery);
  const persona = createAsync(() => personaQuery());
  return (
    <>
      <p class="sub">
        夜ごとに夢を見たか、何件の記憶を変えたか。変えた中身は<A href="/memory">記憶</A>の夢で読む。
      </p>
      <div class="list">
        <For each={pages.rows()}>
          {(d) => (
            <div class="item dream-log-entry">
              <span class="mono sub" style={{ width: "90px" }}>
                {nightLabel(d.at, persona()?.timezone ?? "")}
              </span>
              <div class="grow">
                <div style={{ "font-size": "13px", "overflow-wrap": "anywhere" }}>{d.status}</div>
                <Show when={counts(d)}>
                  <div class="sub">{counts(d)}</div>
                </Show>
              </div>
            </div>
          )}
        </For>
        <Show when={pages.loaded() && pages.rows().length === 0}>
          <p class="empty">まだ夢を見ていない</p>
        </Show>
      </div>
      <PageNav pages={pages} />
    </>
  );
}
