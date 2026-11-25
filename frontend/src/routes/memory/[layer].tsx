import { createAsync, Navigate, useNavigate, useParams, useSearchParams } from "@solidjs/router";
import { createEffect, createSignal, For, Match, Show, Switch } from "solid-js";
import AskAbout from "~/components/AskAbout";
import Markdown from "~/components/Markdown";
import NoteSheet from "~/components/NoteSheet";
import { GROUP_NAMES, MemorySheet } from "~/components/MemoryDetail";
import { DetailLink, EntryRow, SrcTag, usePick } from "~/components/MemoryEntry";
import PendingActions from "~/components/PendingActions";
import { EmptyState, IconButton, Keyed, LinkButton, List, ListRow, Loading, Meter, PageHeader, Section } from "~/components/ui";
import type { Layer, MemoryFile, MemoryItem } from "~/gen/famulen/v1/memory_pb";
import * as icons from "~/lib/icons";
import { layerQuery, visible } from "~/lib/memory";
import { useOwner } from "~/lib/owner";
import { keepScroll } from "~/lib/keepScroll";
import { useParam, withParams } from "~/lib/params";
import { setView } from "~/lib/view";
import { workgroup } from "~/lib/workgroup";

const LAYERS = ["pending", "episodic", "working", "core", "long", "forgotten"];

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
  const pick = usePick();
  return (
    <>
      <PageHeader title="保留中の判断" note="まったく急がない。答えなくてよい。" actions={<FileLink path="pending.md" />} />
      <List>
        <Show when={shown(props.data.items).length > 0} fallback={<EmptyState>いまはない</EmptyState>}>
          <Keyed each={shown(props.data.items)} by={(e) => e.id || e.text}>
            {(e) => (
              <ListRow
                class="memory-row wraps"
                title={<Markdown inline text={e().text} />}
                sub={
                  <>
                    <SrcTag src={e().src} />
                    <span>
                      根拠 {e().attrs.ev ?? "0"} / 要る {e().attrs.need ?? "2"}
                    </span>
                    <Show when={e().attrs.against}>
                      <span>あなたの言葉と食い違う</span>
                    </Show>
                  </>
                }
                onPick={() => pick(e())}
                end={
                  <>
                    <AskAbout what={`保留中の判断：${e().text}`} />
                    <DetailLink item={e()} />
                  </>
                }
              >
                <Show when={e().id}>
                  <div class="item-more">
                    <PendingActions item={e()} />
                  </div>
                </Show>
              </ListRow>
            )}
          </Keyed>
        </Show>
      </List>
      <p class="sub">
        採用と、書いた判断は、あなたの言葉として次の夢が記憶に入れる。却下したものはすぐ忘却に移り、保留に戻せる。何もしなければ保留のまま。
      </p>
      <p class="sub">
        夢のたびに根拠を数え直し、そろえば {props.aide}{" "}
        が自分で決めて長期記憶に移す。話の流れで確かめられるときだけ、ついでに聞く。あなたの言ったことを覆すには、ふつうより多い根拠が要る。
      </p>
    </>
  );
}

function Notes(props: { data: Layer }) {
  const notes = () => shown(props.data.items);
  const hidden = () => props.data.items.length - notes().length;
  const files = () => [...new Set(notes().map((e) => e.file))];
  const [open, setOpen] = createSignal<MemoryItem>();
  return (
    <>
      <PageHeader title="気づき" note="雑に書く受信箱。ここにあるのは夢を待つものだけで、夢が整理したら消える。" />
      <Show when={hidden()}>{(n) => <p class="sub">{`ほかのワークグループの ${n()} 件は出ていない`}</p>}</Show>
      <For each={files()} fallback={<EmptyState>夢を待つ気づきはない</EmptyState>}>
        {(file) => (
          <Section title={file.replace(/^.*\/|\.md$/g, "")} action={<FileLink path={file} />}>
            <List>
              <For each={notes().filter((e) => e.file === file)}>
                {(e) => (
                  <ListRow
                    class="memory-row"
                    lead={<span class="mono sub memory-time">{e.time}</span>}
                    title={<Markdown inline text={e.text} />}
                    sub={<SrcTag src={e.src} />}
                    onPick={() => setOpen(e)}
                    end={
                      <>
                        <AskAbout what={`気づき：${e.text}`} />
                        <IconButton plain icon={icons.listDetails} label={`${e.text} ― 詳細を見る`} title="詳細を見る" onClick={() => setOpen(e)} />
                      </>
                    }
                  />
                )}
              </For>
            </List>
          </Section>
        )}
      </For>
      <NoteSheet note={open()} onClose={() => setOpen(undefined)} />
    </>
  );
}

function Working(props: { data: Layer }) {
  const navigate = useNavigate();
  const href = (name: string) => withParams("/memory/file", { path: `working/${name}.md` });
  return (
    <>
      <PageHeader title="作業記憶" note="いま進めている仕事の途中経過。済めば消える。" />
      <For each={props.data.working} fallback={<EmptyState>いま進めている仕事はない</EmptyState>}>
        {(w) => (
          <article
            class="card stack memory-card"
            onClick={(e) => {
              const selecting = window.getSelection()?.isCollapsed === false;
              if (!selecting && e.target instanceof Element && !e.target.closest("a, button, .md-link")) navigate(href(w.name));
            }}
          >
            <div class="row">
              <h2 class="grow">
                <Markdown inline text={w.title || w.name} />
              </h2>
              <LinkButton small variant="ghost" icon={icons.listDetails} label={`${w.title || w.name} ― 詳細を見る`} href={href(w.name)} />
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
        <Show when={shown(props.data.items).length > 0} fallback={<EmptyState>まだない</EmptyState>}>
          <Keyed each={shown(props.data.items)} by={(e) => e.id || e.text}>
            {(e) => <EntryRow item={e()} />}
          </Keyed>
        </Show>
      </List>
    </>
  );
}

function Long(props: { data: Layer }) {
  let top!: HTMLDivElement;
  keepScroll("memory.long", () => top, "/memory/file");
  const rows = (group: string) =>
    props.data.files
      .filter((f) => f.path.split("/")[1] === group)
      .flatMap((file): { key: string; file: MemoryFile; entry?: MemoryItem }[] =>
        file.entries.length === 0 ? [{ key: file.path, file }] : shown(file.entries).map((entry) => ({ key: file.path, file, entry })),
      );
  return (
    <>
      <div ref={top} />
      <PageHeader title="長期記憶" note="覚えたこと。必要なときに読みに行く。押すと詳細が開く。" />
      <For each={GROUP_NAMES}>
        {([group, label]) => (
          <Section title={label}>
            <List>
              <Show when={rows(group).length > 0} fallback={<EmptyState>まだない</EmptyState>}>
                <Keyed each={rows(group)} by={(r) => r.key}>
                  {(r) => (
                    <Show
                      when={r().entry}
                      fallback={
                        <ListRow
                          leads
                          href={withParams("/memory/file", { path: r().file.path })}
                          title={r().file.title}
                          sub="まだ何も書かれていない"
                        />
                      }
                    >
                      {(e) => <EntryRow item={e()} />}
                    </Show>
                  )}
                </Keyed>
              </Show>
            </List>
          </Section>
        )}
      </For>
    </>
  );
}

function Forgotten(props: { data: Layer; at: string }) {
  const pick = usePick();
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
                onPick={() => pick(e)}
                end={
                  <>
                    <AskAbout what={`忘れた記憶：${e.text}`} label="戻すよう頼む" />
                    <DetailLink item={e} />
                  </>
                }
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
  const [search, setSearch] = useSearchParams<{ item?: string }>();
  const sheet = () => (["pending", "core", "forgotten"].includes(params.layer ?? "") ? (search.item ?? "") : "");
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
                <Notes data={d()} />
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
        <MemorySheet id={sheet()} layer={params.layer ?? ""} onClose={() => setSearch({ item: undefined }, { scroll: false })} />
      </Show>
    </Show>
  );
}
