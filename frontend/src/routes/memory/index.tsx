import { A, createAsync, useSearchParams } from "@solidjs/router";
import { For, Show } from "solid-js";
import Icon from "~/components/Icon";
import DreamNow from "~/components/DreamNow";
import type { Dream } from "~/gen/famulen/v1/memory_pb";
import { personaQuery } from "~/lib/data";
import * as icons from "~/lib/icons";
import { dreamSettingsQuery, dreamsQuery, nightLabel } from "~/lib/memory";
import { setView } from "~/lib/view";

const OPS: [string, string][] = [
  ["足した", "#2a78d6"],
  ["直した", "#eb6834"],
  ["置き換えた", "#1baf7a"],
  ["忘れた", "#eda100"],
];

const OP_LABELS: Record<string, string> = {
  added: "足した",
  edited: "直した",
  replaced: "置き換えた",
  archived: "忘れた",
  confirmed: "確かめた",
  pending_added: "保留にした",
  moved: "移した",
  deleted: "消した",
};

const BAR_HEIGHT = 150;

function opCounts(d: Dream): number[] {
  return [d.added, d.edited, d.replaced, d.archived];
}

export default function Dreams() {
  setView("記憶 · 夢");
  const [params] = useSearchParams();
  const dreams = createAsync(() => dreamsQuery());
  const plan = createAsync(() => dreamSettingsQuery());
  const persona = createAsync(() => personaQuery());
  const tz = () => persona()?.timezone ?? "UTC";
  const nights = () => dreams()?.dreams ?? [];
  const index = () => {
    const wanted = params.night;
    const found = typeof wanted === "string" ? nights().findIndex((n) => n.id.toString() === wanted) : -1;
    return found >= 0 ? found : nights().length - 1;
  };
  const night = () => nights()[index()];
  const max = () => Math.max(5, ...nights().map((n) => opCounts(n).reduce((a, b) => a + b, 0)));
  const href = (n: Dream) => `/memory?night=${n.id}`;

  return (
    <>
      <div class="row" style={{ "align-items": "baseline" }}>
        <h1>夢</h1>
        <span class="sub">
          次は {dreams()?.next}。
          <Show when={plan()?.earlyAt}>{(n) => `エピソード記憶の気づきが ${n()} 件たまれば早めに見る。`}</Show>
          いま待っている気づき {dreams()?.waitingNotes ?? 0} 件
        </span>
        <span class="row" style={{ "margin-left": "auto" }}>
          <DreamNow linkWhy />
        </span>
      </div>
      <section class="card" style={{ display: "flex", "flex-direction": "column", gap: "12px" }}>
        <div class="row">
          <h2>夢ごとの変化（この 2 週間）</h2>
          <div class="legend">
            <For each={OPS}>
              {([l, c]) => (
                <span>
                  <i style={{ background: c }} />
                  {l}
                </span>
              )}
            </For>
          </div>
        </div>
        <Show when={nights().length > 0} fallback={<p class="empty">まだ夢を見ていない</p>}>
          <div class="row" style={{ "flex-wrap": "nowrap", "align-items": "stretch" }}>
            <div
              style={{
                display: "flex",
                "flex-direction": "column",
                "justify-content": "space-between",
                height: `${BAR_HEIGHT}px`,
                "font-size": "11px",
                color: "var(--muted)",
                width: "22px",
                "text-align": "right",
              }}
            >
              <span>{max()}</span>
              <span>0</span>
            </div>
            <div
              class="bars grow"
              style={{
                height: `${BAR_HEIGHT}px`,
                "grid-template-columns": `repeat(${nights().length},minmax(0,1fr))`,
                "background-image": "linear-gradient(to bottom,#EEECE5 1px,transparent 1px)",
                "background-size": "100% 50px",
              }}
            >
              <For each={nights()}>
                {(n, i) => {
                  const counts = opCounts(n);
                  return (
                    <A
                      class={`bar${i() === index() ? " on" : ""}`}
                      href={href(n)}
                      title={`${nightLabel(n.at, tz())} · 足した ${counts[0]} · 直した ${counts[1]} · 置き換えた ${counts[2]} · 忘れた ${counts[3]}`}
                    >
                      <For each={counts}>
                        {(c, k) => (
                          <Show when={c > 0}>
                            <span
                              style={{
                                height: `${Math.floor((c * BAR_HEIGHT) / max())}px`,
                                background: OPS[k()][1],
                              }}
                            />
                          </Show>
                        )}
                      </For>
                    </A>
                  );
                }}
              </For>
            </div>
          </div>
          <p class="sub">棒を押すと、その夜の夢を下に開く。確かめただけの変更は数に入れない。</p>
        </Show>
      </section>
      <Show when={night()}>
        {(n) => (
          <section class="card" style={{ display: "flex", "flex-direction": "column", gap: "12px" }}>
            <div class="row">
              <Show when={index() > 0}>
                <A class="btn small" href={href(nights()[index() - 1])} aria-label="前の夢">
                  <Icon svg={icons.chevronLeft} size={14} />
                </A>
              </Show>
              <h2>
                {index() + 1 === nights().length ? "最後の夢 · " : ""}
                {nightLabel(n().at, tz())}
              </h2>
              <Show when={index() + 1 < nights().length}>
                <A class="btn small" href={href(nights()[index() + 1])} aria-label="次の夢">
                  <Icon svg={icons.chevronRight} size={14} />
                </A>
              </Show>
              <span class="sub">{n().status}</span>
              <Show when={n().commit}>
                <span class="mono sub" style={{ "margin-left": "auto" }}>
                  commit {n().commit.slice(0, 7)}
                </span>
              </Show>
            </div>
            <div style={{ display: "grid", "grid-template-columns": "repeat(6,minmax(0,1fr))", gap: "8px" }}>
              <For
                each={
                  [
                    ["足した", n().added],
                    ["直した", n().edited],
                    ["置き換えた", n().replaced],
                    ["確かめた", n().confirmed],
                    ["忘れた", n().archived],
                    ["保留にした", n().pendingAdded],
                  ] as const
                }
              >
                {([label, count]) => (
                  <div class="card" style={{ padding: "10px 12px" }}>
                    <div class="mono" style={{ "font-size": "20px" }}>
                      {count}
                    </div>
                    <div class="sub">{label}</div>
                  </div>
                )}
              </For>
            </div>
            <div class="list" style={{ border: 0, padding: 0 }}>
              <For each={n().changes} fallback={<p class="empty">変更はなかった</p>}>
                {(c) => (
                  <div class="item" style={{ "align-items": "flex-start" }}>
                    <span class="tag">{OP_LABELS[c.op] ?? c.op}</span>
                    <div class="grow">
                      <div style={{ "font-size": "13px" }}>{c.text}</div>
                      <div class="sub">
                        {c.fromFile ? `${c.fromFile} → ${c.file}` : c.file}
                        {c.reason ? ` · ${c.reason}` : ""}
                        {c.previous ? ` · 前は「${c.previous}」` : ""}
                      </div>
                    </div>
                  </div>
                )}
              </For>
            </div>
            <p class="sub">
              夢は 1 つのコミットにまとまる。全体の書き直しはせず、項目ごとの小さな変更だけを当てる。ファイルを半分より小さくする変更と、あなたの言ったことを消す変更は当てない。
            </p>
          </section>
        )}
      </Show>
    </>
  );
}
