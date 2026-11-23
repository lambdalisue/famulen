import { A, createAsync, Navigate, useParams, useSearchParams } from "@solidjs/router";
import { createEffect, For, Match, Show, Switch } from "solid-js";
import Icon from "~/components/Icon";
import Markdown from "~/components/Markdown";
import { EntryRow, SrcTag } from "~/components/MemoryEntry";
import type { Layer } from "~/gen/famulen/v1/memory_pb";
import { workgroup } from "~/lib/workgroup";
import * as icons from "~/lib/icons";
import { layerQuery, visible } from "~/lib/memory";
import { useOwner } from "~/lib/owner";
import { oneParam } from "~/lib/params";
import { setSelection, setView } from "~/lib/view";

const LONG_GROUPS: [string, string][] = [
  ["semantic", "意味記憶"],
  ["procedural", "手続き記憶"],
  ["prospective", "展望記憶"],
];

const LAYERS = ["pending", "episodic", "working", "core", "long", "forgotten"];

const STATES: Record<string, string> = { waiting: "夢を待つ", applied: "すぐ入れた", record: "記録へ" };

function FileLink(props: { path: string }) {
  return (
    <A class="btn small" style={{ "margin-left": "auto" }} href={`/memory/file?path=${encodeURIComponent(props.path)}`}>
      <Icon svg={icons.pencil} size={12} />
      読む・書く・経緯
    </A>
  );
}

/** Buttons for the dates or months a layer can show, newest first. */
function Periods(props: { periods: string[]; at: string; layer: string; label: (p: string) => string }) {
  return (
    <div class="row">
      <For each={props.periods.slice(0, 14)}>
        {(p) => (
          <A class={`btn small${p === props.at ? " primary" : ""}`} href={`/memory/${props.layer}?at=${p}`}>
            {props.label(p)}
          </A>
        )}
      </For>
    </div>
  );
}

function Pending(props: { data: Layer; aide: string }) {
  const items = () => props.data.items.filter((e) => visible(e, workgroup()));
  return (
    <>
      <div class="row">
        <h1>保留中の判断</h1>
        <span class="sub">まったく急がない。答えなくてよい</span>
        <FileLink path="pending.md" />
      </div>
      <div class="list">
        <For each={items()} fallback={<p class="empty">いまはない</p>}>
          {(e) => (
            <div class="item">
              <div class="grow">
                <div style={{ "font-size": "14px" }}><Markdown inline text={e.text} /></div>
                <div class="sub">
                  根拠 {e.attrs.ev ?? "0"} / 要る {e.attrs.need ?? "2"}
                  {e.attrs.against ? " · あなたの言葉と食い違う" : ""}
                </div>
              </div>
              <button type="button" class="btn small" onClick={() => setSelection(`保留中の判断：${e.text}`)}>
                答える
              </button>
            </div>
          )}
        </For>
      </div>
      <p class="sub">
        夢のたびに根拠を数え直し、そろえば {props.aide}{" "}
        が自分で決めて長期記憶に移す。話の流れで確かめられるときだけ、ついでに聞く。あなたの言ったことを覆すには、ふつうより多い根拠が要る。
      </p>
    </>
  );
}

function Episodic(props: { data: Layer; at: string }) {
  const date = () => props.at || props.data.periods[0] || "";
  const items = () => props.data.items.filter((e) => visible(e, workgroup()));
  return (
    <>
      <div class="row">
        <h1>エピソード記憶</h1>
        <span class="sub">昼の間はここに書き留めるだけ。整理は夢の中で行う</span>
        <Show when={date()}>
          <FileLink path={`episodic/${date()}.md`} />
        </Show>
      </div>
      <Periods
        periods={props.data.periods}
        at={date()}
        layer="episodic"
        label={(d) => `${Number(d.slice(5, 7))}/${Number(d.slice(8, 10))}`}
      />
      <div class="list">
        <For each={items()} fallback={<p class="empty">まだない</p>}>
          {(e) => (
            <div class="item">
              <span class="mono" style={{ "font-size": "12px", width: "44px" }}>
                {e.time}
              </span>
              <SrcTag src={e.src} />
              <div
                class="grow"
                style={{ "font-size": "14px", cursor: "pointer" }}
                onClick={() => setSelection(`記憶：${e.text}`)}
              >
                <Markdown inline text={e.text} />
              </div>
              <span class="sub">{STATES[e.attrs.state ?? ""] ?? ""}</span>
            </div>
          )}
        </For>
      </div>
    </>
  );
}

function Working(props: { data: Layer }) {
  return (
    <>
      <div class="row">
        <h1>作業記憶</h1>
        <span class="sub">いま進めている仕事の途中経過。済めば消える</span>
      </div>
      <For each={props.data.working} fallback={<p class="empty">いま進めている仕事はない</p>}>
        {(w) => (
          <article class="card" style={{ display: "flex", "flex-direction": "column", gap: "8px" }}>
            <div class="row">
              <h2 class="grow"><Markdown inline text={w.title || w.name} /></h2>
              <FileLink path={`working/${w.name}.md`} />
            </div>
            <div
              style={{
                display: "grid",
                "grid-template-columns": "96px minmax(0,1fr)",
                "row-gap": "4px",
                "font-size": "13px",
                "line-height": 1.6,
              }}
            >
              <span class="sub">ここまで</span>
              <div><Markdown inline text={w.done} /></div>
              <span class="sub">次にやる</span>
              <div><Markdown inline text={w.next} /></div>
              <span class="sub">待っている</span>
              <div><Markdown inline text={w.waiting} /></div>
              <span class="sub">手がかり</span>
              <div class="mono" style={{ "font-size": "12px" }}>
                <Markdown inline text={w.clues} />
              </div>
            </div>
          </article>
        )}
      </For>
    </>
  );
}

function Core(props: { data: Layer }) {
  const items = () => props.data.items.filter((e) => visible(e, workgroup()));
  const lines = () => props.data.items.length;
  const limit = () => Number(/\/\s*(\d+)/.exec(props.data.layer?.count ?? "")?.[1] ?? "150");
  return (
    <>
      <div class="row">
        <h1>心得</h1>
        <span class="sub">毎回の会話の最初に必ず読む。小さく保つ。ここにあるものも長期記憶の一部</span>
        <FileLink path="core.md" />
      </div>
      <div>
        <div class="row" style={{ "justify-content": "space-between", "font-size": "12px" }}>
          <span>大きさ</span>
          <span class="mono">
            {lines()} / {limit()} 行
          </span>
        </div>
        <div class="meter">
          <div style={{ width: `${Math.min(100, Math.floor((lines() * 100) / limit()))}%` }} />
        </div>
        <p class="sub">上限を超えそうになると、夢の中で使われていないものから心得を外す。外しても長期記憶には残る</p>
      </div>
      <div class="list">
        <For each={items()} fallback={<p class="empty">まだない</p>}>
          {(e) => <EntryRow item={e} />}
        </For>
      </div>
    </>
  );
}

function Long(props: { data: Layer }) {
  const files = (group: string) => props.data.files.filter((f) => f.path.split("/")[1] === group);
  return (
    <>
      <div class="row">
        <h1>長期記憶</h1>
        <span class="sub">覚えたこと。必要なときに読みに行く</span>
      </div>
      <For each={LONG_GROUPS}>
        {([group, label]) => (
          <section style={{ display: "flex", "flex-direction": "column", gap: "8px" }}>
            <h3>{label}</h3>
            <For each={files(group)} fallback={<p class="empty">まだない</p>}>
              {(f) => (
                <div class="card" style={{ display: "flex", "flex-direction": "column", gap: "6px" }}>
                  <div class="row">
                    <b class="grow"><Markdown inline text={f.title.replace(/\.md$/, "")} /></b>
                    <FileLink path={f.path} />
                  </div>
                  <div class="list" style={{ border: 0, padding: 0 }}>
                    <For each={f.entries.filter((e) => visible(e, workgroup()))}>{(e) => <EntryRow item={e} />}</For>
                  </div>
                </div>
              )}
            </For>
          </section>
        )}
      </For>
    </>
  );
}

function Forgotten(props: { data: Layer; at: string }) {
  const month = () => props.at || props.data.periods[0] || "";
  const items = () => props.data.items.filter((e) => visible(e, workgroup()));
  return (
    <>
      <div class="row">
        <h1>忘却</h1>
        <span class="sub">古くなった・置き換わった記憶。ふだんは読まないが、消してはいないので戻せる</span>
      </div>
      <Show when={month()} fallback={<p class="empty">まだない</p>}>
        <Periods periods={props.data.periods} at={month()} layer="forgotten" label={(m) => m} />
        <section style={{ display: "flex", "flex-direction": "column", gap: "6px" }}>
          <div class="row">
            <h3 class="grow">{month()}</h3>
            <FileLink path={`forgotten/${month()}.md`} />
          </div>
          <div class="list">
            <For each={items()} fallback={<p class="empty">まだない</p>}>
              {(e) => (
                <div class="item">
                  <div class="grow">
                    <div style={{ "font-size": "14px", color: "#4D4C47", "text-decoration": "line-through" }}>
                      <Markdown inline text={e.text} />
                    </div>
                    <div class="sub">
                      <Markdown inline text={e.attrs.reason ?? ""} /> · {e.attrs.forgot ?? ""}
                    </div>
                  </div>
                  <button type="button" class="btn small" onClick={() => setSelection(`忘れた記憶：${e.text}`)}>
                    戻すよう頼む
                  </button>
                </div>
              )}
            </For>
          </div>
        </section>
      </Show>
    </>
  );
}

export default function MemoryLayer() {
  const params = useParams();
  const [search] = useSearchParams();
  const at = () => oneParam(search.at);
  const known = () => LAYERS.includes(params.layer ?? "");
  const data = createAsync(async () => (known() ? layerQuery(params.layer ?? "", at()) : undefined));
  const owner = useOwner();
  const aide = owner.aide;
  createEffect(() => {
    const label = data()?.layer?.label;
    if (label) setView("記憶", label);
  });
  return (
    <Show when={params.layer !== "dream"} fallback={<Navigate href="/memory" />}>
    <Show when={known()} fallback={<p class="empty">そういう層はない</p>}>
    <Show when={data()}>
      {(d) => (
        <Switch>
          <Match when={params.layer === "pending"}>
            <Pending data={d()} aide={aide()} />
          </Match>
          <Match when={params.layer === "episodic"}>
            <Episodic data={d()} at={at()} />
          </Match>
          <Match when={params.layer === "working"}>
            <Working data={d()} />
          </Match>
          <Match when={params.layer === "core"}>
            <Core data={d()} />
          </Match>
          <Match when={params.layer === "long"}>
            <Long data={d()} />
          </Match>
          <Match when={params.layer === "forgotten"}>
            <Forgotten data={d()} at={at()} />
          </Match>
        </Switch>
      )}
    </Show>
    </Show>
    </Show>
  );
}
