import { dateTimeText } from "~/lib/dates";
import { A, createAsync, query } from "@solidjs/router";
import { For, Show } from "solid-js";
import type { Dream } from "~/gen/famulen/v1/memory_pb";
import { api } from "~/lib/api";
import { useOwner } from "~/lib/owner";
import { createPages, PAGE, PageNav } from "./Pages";
import { EmptyState, List } from "~/components/ui";

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
    ["タスクにした", d.taskMade],
    ["古いかもの印", d.doubted],
  ];
  return parts
    .filter(([, n]) => n > 0)
    .map(([label, n]) => `${label} ${n}`)
    .join(" · ");
}

/** Each night the dream ran: how it went and how much it changed. */
export default function DreamLog() {
  const pages = createPages(dreamLogQuery);
  const owner = useOwner();
  return (
    <>
      <p class="sub">
        夜ごとに夢を見たか、何件の記憶と記録を変えたか。変えた中身は<A href="/memory">記憶</A>の夢で読む。
      </p>
      <List>
        <For each={pages.rows()}>
          {(d) => (
            <div role="listitem" class="item dream-log-entry">
              <span class="mono sub when">
                {dateTimeText(d.at, owner.tz())}
              </span>
              <div class="grow">
                <div>{d.status}</div>
                <Show when={counts(d)}>
                  <div class="sub">{counts(d)}</div>
                </Show>
              </div>
            </div>
          )}
        </For>
        <Show when={pages.loaded() && pages.rows().length === 0}>
          <EmptyState>まだ夢を見ていない</EmptyState>
        </Show>
      </List>
      <PageNav pages={pages} />
    </>
  );
}
