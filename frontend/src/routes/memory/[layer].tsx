import { createAsync, Navigate, useParams } from "@solidjs/router";
import { createEffect, For, Match, Show, Switch } from "solid-js";
import AskAbout from "~/components/AskAbout";
import Markdown from "~/components/Markdown";
import { EntryRow, SrcTag } from "~/components/MemoryEntry";
import { EmptyState, LinkButton, List, ListRow, Loading, Meter, PageHeader, Section } from "~/components/ui";
import type { Layer, MemoryItem } from "~/gen/famulen/v1/memory_pb";
import * as icons from "~/lib/icons";
import { layerQuery, visible, waitsForDream } from "~/lib/memory";
import { useOwner } from "~/lib/owner";
import { useParam, withParams } from "~/lib/params";
import { setView } from "~/lib/view";
import { workgroup } from "~/lib/workgroup";

const LONG_GROUPS: [string, string][] = [
  ["semantic", "意味記憶"],
  ["procedural", "手続き記憶"],
  ["prospective", "展望記憶"],
];

const LAYERS = ["pending", "episodic", "working", "core", "long", "forgotten"];

const STATES: Record<string, string> = { applied: "すぐ入れた", record: "記録へ" };

/** Where an episodic note has got to; a dream marks what it is done with, kept or not, and leaves the note in its file. */
const stateText = (e: MemoryItem) => {
  if (waitsForDream(e)) return "夢を待つ";
  return e.attrs.dreamt === undefined ? (STATES[e.attrs.state ?? ""] ?? "") : "夢が済んだ";
};

/** Where a memory's file is read, written and its history seen. */
function FileLink(props: { path: string }) {
  return (
    <LinkButton small icon={icons.pencil} href={withParams("/memory/file", { path: props.path })}>
      読む・書く・経緯
    </LinkButton>
  );
}

/** The dates or months a layer can show, newest first. */
function Periods(props: { periods: string[]; at: string; layer: string; label: (p: string) => string }) {
  return (
    <nav class="row" aria-label="期間">
      <For each={props.periods.slice(0, 14)}>
        {(p) => (
          <a
            class={`btn small${p === props.at ? " primary" : ""}`}
            href={`/memory/${props.layer}?at=${p}`}
            aria-current={p === props.at ? "true" : undefined}
          >
            {props.label(p)}
          </a>
        )}
      </For>
    </nav>
  );
}

/** The items of a layer that belong to the workgroup on screen. */
const shown = (items: MemoryItem[]) => items.filter((e) => visible(e, workgroup()));

function Pending(props: { data: Layer; aide: string }) {
  return (
    <>
      <PageHeader title="保留中の判断" note="まったく急がない。答えなくてよい。" actions={<FileLink path="pending.md" />} />
      <List>
        <For each={shown(props.data.items)} fallback={<EmptyState>いまはない</EmptyState>}>
          {(e) => (
            <ListRow
              class="memory-row"
              title={<Markdown inline text={e.text} />}
              sub={
                <>
                  <span>
                    根拠 {e.attrs.ev ?? "0"} / 要る {e.attrs.need ?? "2"}
                  </span>
                  <Show when={e.attrs.against}>
                    <span>あなたの言葉と食い違う</span>
                  </Show>
                </>
              }
              end={<AskAbout what={`保留中の判断：${e.text}`} label="答える" />}
            />
          )}
        </For>
      </List>
      <p class="sub">
        夢のたびに根拠を数え直し、そろえば {props.aide}{" "}
        が自分で決めて長期記憶に移す。話の流れで確かめられるときだけ、ついでに聞く。あなたの言ったことを覆すには、ふつうより多い根拠が要る。
      </p>
    </>
  );
}

function Episodic(props: { data: Layer; at: string }) {
  const date = () => props.at || props.data.periods[0] || "";
  const all = () => props.data.items;
  const notes = () => shown(all());
  const hidden = () => all().length - notes().length;
  return (
    <>
      <PageHeader
        title="エピソード記憶"
        note="昼の間はここに書き留めるだけ。整理は夢の中で行う。"
        actions={
          <Show when={date()}>
            <FileLink path={`episodic/${date()}.md`} />
          </Show>
        }
      />
      <Periods
        periods={props.data.periods}
        at={date()}
        layer="episodic"
        label={(d) => `${Number(d.slice(5, 7))}/${Number(d.slice(8, 10))}`}
      />
      <Show when={all().length}>
        <p class="sub">
          {`この日の ${all().length} 件のうち、夢を待つ ${all().filter(waitsForDream).length} 件`}
          <Show when={hidden()}>{(n) => `。ほかのワークグループの ${n()} 件は出ていない`}</Show>
        </p>
      </Show>
      <List>
        <For each={notes()} fallback={<EmptyState>まだない</EmptyState>}>
          {(e) => (
            <ListRow
              class="memory-row"
              lead={<span class="mono sub memory-time">{e.time}</span>}
              title={<Markdown inline text={e.text} />}
              sub={
                <>
                  <SrcTag src={e.src} />
                  <span>{stateText(e)}</span>
                </>
              }
              end={<AskAbout what={`記憶：${e.text}`} />}
            />
          )}
        </For>
      </List>
    </>
  );
}

function Working(props: { data: Layer }) {
  return (
    <>
      <PageHeader title="作業記憶" note="いま進めている仕事の途中経過。済めば消える。" />
      <For each={props.data.working} fallback={<EmptyState>いま進めている仕事はない</EmptyState>}>
        {(w) => (
          <article class="card stack">
            <div class="row">
              <h2 class="grow">
                <Markdown inline text={w.title || w.name} />
              </h2>
              <FileLink path={`working/${w.name}.md`} />
            </div>
            <dl class="memory-working">
              <dt>ここまで</dt>
              <dd>
                <Markdown inline text={w.done} />
              </dd>
              <dt>次にやる</dt>
              <dd>
                <Markdown inline text={w.next} />
              </dd>
              <dt>待っている</dt>
              <dd>
                <Markdown inline text={w.waiting} />
              </dd>
              <dt>手がかり</dt>
              <dd class="mono">
                <Markdown inline text={w.clues} />
              </dd>
            </dl>
          </article>
        )}
      </For>
    </>
  );
}

function Core(props: { data: Layer }) {
  const lines = () => props.data.items.length;
  const limit = () => Number(/\/\s*(\d+)/.exec(props.data.layer?.count ?? "")?.[1] ?? "150");
  return (
    <>
      <PageHeader
        title="心得"
        note="毎回の会話の最初に必ず読む。小さく保つ。ここにあるものも長期記憶の一部。"
        actions={<FileLink path="core.md" />}
      />
      <Meter label="大きさ" value={lines()} max={limit()} unit="行" />
      <p class="sub">上限を超えそうになると、夢の中で使われていないものから心得を外す。外しても長期記憶には残る。</p>
      <List>
        <For each={shown(props.data.items)} fallback={<EmptyState>まだない</EmptyState>}>
          {(e) => <EntryRow item={e} />}
        </For>
      </List>
    </>
  );
}

function Long(props: { data: Layer }) {
  const files = (group: string) => props.data.files.filter((f) => f.path.split("/")[1] === group);
  return (
    <>
      <PageHeader title="長期記憶" note="覚えたこと。必要なときに読みに行く。" />
      <For each={LONG_GROUPS}>
        {([group, label]) => (
          <section class="stack">
            <h3>{label}</h3>
            <For each={files(group)} fallback={<EmptyState>まだない</EmptyState>}>
              {(f) => (
                <Section title={<Markdown inline text={f.title.replace(/\.md$/, "")} />} action={<FileLink path={f.path} />}>
                  <List>
                    <For each={shown(f.entries)}>{(e) => <EntryRow item={e} />}</For>
                  </List>
                </Section>
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
  return (
    <>
      <PageHeader
        title="忘却"
        note="古くなった・置き換わった記憶。ふだんは読まないが、消してはいないので戻せる。"
        actions={
          <Show when={month()}>
            <FileLink path={`forgotten/${month()}.md`} />
          </Show>
        }
      />
      <Show when={month()} fallback={<EmptyState>まだない</EmptyState>}>
        <Periods periods={props.data.periods} at={month()} layer="forgotten" label={(m) => m} />
        <List>
          <For each={shown(props.data.items)} fallback={<EmptyState>まだない</EmptyState>}>
            {(e) => (
              <ListRow
                class="memory-row"
                title={
                  <span class="struck">
                    <Markdown inline text={e.text} />
                  </span>
                }
                sub={
                  <>
                    <Markdown inline text={e.attrs.reason ?? ""} />
                    <span>{e.attrs.forgot ?? ""}</span>
                  </>
                }
                end={<AskAbout what={`忘れた記憶：${e.text}`} label="戻すよう頼む" />}
              />
            )}
          </For>
        </List>
      </Show>
    </>
  );
}

export default function MemoryLayer() {
  const params = useParams();
  const at = useParam("at");
  const owner = useOwner();
  const known = () => LAYERS.includes(params.layer ?? "");
  const data = createAsync(async () => (known() ? layerQuery(params.layer ?? "", at()) : undefined));
  createEffect(() => {
    const label = data()?.layer?.label;
    if (label) setView("記憶", label);
  });
  return (
    <Show when={params.layer !== "dream"} fallback={<Navigate href="/memory" />}>
      <Show when={known()} fallback={<EmptyState>そういう層はない</EmptyState>}>
        <Show when={data()} fallback={<Loading />}>
          {(d) => (
            <Switch>
              <Match when={params.layer === "pending"}>
                <Pending data={d()} aide={owner.aide()} />
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
