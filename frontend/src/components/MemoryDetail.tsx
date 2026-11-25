import { Dialog } from "@ark-ui/solid/dialog";
import { createAsync } from "@solidjs/router";
import { createMemo, ErrorBoundary, For, Match, Show, Suspense, Switch } from "solid-js";
import AskAbout from "~/components/AskAbout";
import Markdown from "~/components/Markdown";
import { SrcTag, spanOf, workgroupIds } from "~/components/MemoryEntry";
import { Copied, Facts, Source } from "~/components/MemoryFacts";
import PendingActions from "~/components/PendingActions";
import { Button, EmptyState, ErrorState, LinkButton, List, ListRow, Loading, SideSheet, Tag } from "~/components/ui";
import type { FileChange, MemoryDetail as Detail } from "~/gen/famulen/v1/memory_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { workgroupsQuery } from "~/lib/data";
import { dateTimeText } from "~/lib/dates";
import * as icons from "~/lib/icons";
import { dreamsQuery, itemHref, memoryQuery, splitMessage, splitSources } from "~/lib/memory";
import { useOwner } from "~/lib/owner";
import { withParams } from "~/lib/params";
import { hrefOf, KIND_NAMES, workgroupNames } from "~/lib/records";
import { OP_LABELS } from "~/lib/dreams";

const LAYER_NAMES: Record<string, string> = { core: "心得", pending: "保留中の判断", forgotten: "忘却" };

export const GROUP_NAMES: [string, string][] = [
  ["semantic", "意味記憶"],
  ["procedural", "手続き記憶"],
  ["prospective", "展望記憶"],
];

const HOW: Record<string, string> = {
  supersedes: "置き換えた記憶",
  superseded: "これを置き換えた記憶",
  against: "食い違う記憶",
};

/** The attributes shown under names of their own. */
const KNOWN = new Set([
  ..."id src from to ev need uses last workgroup party person by supersedes against forgot reason was judged".split(" "),
]);

const MAKING = new Set(["added", "replaced", "pending_added"]);

/** Whose change a commit is, as the owner reads it. */
function useAuthor() {
  const owner = useOwner();
  return (author: string) => (author === "Famulen" ? owner.aide() : author === "Owner" ? "あなた" : author);
}

export function History(props: { changes: FileChange[] }) {
  const owner = useOwner();
  const named = useAuthor();
  return (
    <List label="更新の履歴">
      <For each={props.changes} fallback={<EmptyState>まだ経緯はない</EmptyState>}>
        {(c) => (
          <ListRow
            title={<span class="memory-change">{splitMessage(c.message).text}</span>}
            sub={
              <>
                <Tag tone={c.author === "Owner" ? "said" : "aide"}>{named(c.author)}</Tag>
                <span>{dateTimeText(c.at, owner.tz())}</span>
                <code>{c.commit}</code>
              </>
            }
          />
        )}
      </For>
    </List>
  );
}

function Nights(props: { detail: Detail }) {
  const owner = useOwner();
  const dreams = createAsync(() => dreamsQuery());
  const shown = (id: bigint) => dreams()?.dreams.some((d) => d.id === id);
  return (
    <For each={props.detail.nights}>
      {(n) => (
        <div class="memory-night">
          <div class="row">
            <Tag>{OP_LABELS[n.op] ?? n.op}</Tag>
            <Show when={shown(n.dream)} fallback={<span>{dateTimeText(n.at, owner.tz())} の夢</span>}>
              <a href={`/memory?night=${n.dream}`}>{dateTimeText(n.at, owner.tz())} の夢</a>
            </Show>
          </div>
          <Show when={n.reason}>
            <div class="sub">
              <Markdown inline text={n.reason} />
            </div>
          </Show>
          <Show when={n.previous}>
            <div class="sub">
              前は「
              <Markdown inline text={n.previous} />」
            </div>
          </Show>
          <For each={n.basis}>
            {(b) => (
              <div class="memory-basis">
                <div>
                  <Markdown inline text={b.text} />
                </div>
                <div class="item-sub">
                  <SrcTag src={b.src} />
                  <span>
                    気づき {b.date} {b.time}
                  </span>
                  <Show when={b.origin}>
                    <Source text={b.origin} />
                  </Show>
                </div>
              </div>
            )}
          </For>
        </div>
      )}
    </For>
  );
}

/** Everything that is known of a memory. `history` adds what changed it. */
export default function MemoryDetail(props: { detail: Detail; history?: boolean; onGone?: () => void }) {
  const owner = useOwner();
  const workgroups = createAsync(() => workgroupsQuery());
  const d = () => props.detail;
  const item = () => props.detail.item;
  const attrs = () => item()?.attrs ?? {};
  const kind = () => GROUP_NAMES.find(([g]) => g === d().group)?.[1] ?? LAYER_NAMES[d().layer] ?? "";
  const named = useAuthor();
  const parts = createMemo(() => splitSources(d().prose));
  /** What its prose names, and what the commits that changed it do. */
  const sources = createMemo(() => [...new Set([...parts().sources, ...d().history.flatMap((c) => splitMessage(c.message).sources)])]);
  /** Whether a dream wrote it, as far as the nights tell; the newest to bring it in counts. */
  const dreamt = () => d().nights.find((n) => MAKING.has(n.op));
  const first = () => d().created;
  const writer = () => {
    if (attrs().by === "lookout") return "見張りの気づきから、夢が書いた";
    if (dreamt()) return "夢が書いた";
    return named(first()?.author ?? "");
  };
  const written = () => {
    const at = dreamt()?.at ?? first()?.at;
    return at ? dateTimeText(at, owner.tz()) : "";
  };
  const chips = () => workgroupIds(attrs());
  const others = () =>
    Object.entries(attrs())
      .filter(([k, v]) => !KNOWN.has(k) && v)
      .map(([k, v]): [string, string] => [k, v]);
  const restore = async () => {
    const id = item()?.id ?? "";
    if (await act(() => api.memory.restoreMemory({ id }), "保留に戻した")) props.onGone?.();
  };
  return (
    <div class="memory-detail">
      <Show when={item()}>
        {(i) => (
          <>
            <p class="memory-summary" classList={{ struck: d().layer === "forgotten" }}>
              <Markdown inline text={i().text} links={d().links} />
            </p>
            <div class="row">
              <SrcTag src={i().src} />
              <Tag>{kind()}</Tag>
              <span class="grow" />
              <AskAbout what={`${kind() || "記憶"}：${i().text}`} />
            </div>
            <Show when={d().layer === "pending"}>
              <PendingActions item={i()} onGone={props.onGone} />
            </Show>
            <Show when={d().layer === "forgotten" && attrs().was === "pending.md"}>
              <div>
                <Button small icon={icons.arrowBackUp} onClick={() => void restore()}>
                  保留に戻す
                </Button>
              </div>
            </Show>
            <Show when={parts().prose && parts().prose !== i().text}>
              <div class="rec-prose">
                <Markdown text={parts().prose} links={d().links} attributes />
              </div>
            </Show>
            <Facts
              rows={[
                ["書き手", writer()],
                ["書いた日時", written()],
                ["有効な期間", spanOf(attrs())],
                ["根拠の数", attrs().need ? `${attrs().ev ?? 0}（決めるには ${attrs().need}）` : attrs().ev],
                ["使った回数", attrs().uses && `${attrs().uses} 回${attrs().last ? `（最後は ${attrs().last}）` : ""}`],
                ["人", attrs().person],
                [
                  "ワークグループ",
                  chips().length > 0 && (
                    <span class="chips-line">
                      <For each={chips()}>{(c) => <Tag class="wg">{workgroupNames(workgroups(), [c]) || c}</Tag>}</For>
                    </span>
                  ),
                ],
                ["忘れた日", attrs().forgot],
                ["忘れたわけ", attrs().reason && <Markdown inline text={attrs().reason} />],
                ["元の場所", attrs().was && <code>{attrs().was}</code>],
                ...others().map(([k, v]): [string, string] => [k, v]),
                ["ID", i().id && <Copied text={i().id} what="ID" />],
                [
                  "ファイル",
                  i().file && (
                    <Copied text={i().file} what="ファイルの場所" href={withParams("/memory/file", { path: i().file })} />
                  ),
                ],
              ]}
            />
            <Show when={d().links.length > 0}>
              <h3>関わる記録</h3>
              <List>
                <For each={d().links}>
                  {(l) => (
                    <Show
                      when={!l.missing && hrefOf(l.kind, l.id)}
                      fallback={<ListRow title={<span class="struck">{l.title || l.id}</span>} sub="今はない" />}
                    >
                      {(href) => <ListRow leads href={href()} title={l.title} sub={[KIND_NAMES[l.kind] ?? l.kind, l.code].filter(Boolean).join(" · ")} />}
                    </Show>
                  )}
                </For>
              </List>
            </Show>
            <Show when={d().related.length > 0}>
              <h3>関わる記憶</h3>
              <List>
                <For each={d().related}>
                  {(r) => (
                    <Show when={r.item}>
                      {(to) => <ListRow leads href={itemHref(to())} title={to().text} sub={HOW[r.how]} />}
                    </Show>
                  )}
                </For>
              </List>
            </Show>
            <h3>出どころ</h3>
            <Show
              when={sources().length > 0 || d().nights.length > 0}
              fallback={<EmptyState>根拠は残っていない</EmptyState>}
            >
              <Show when={sources().length > 0}>
                <ul class="memory-sources">
                  <For each={sources()}>
                    {(s) => (
                      <li>
                        <Source text={s} />
                      </li>
                    )}
                  </For>
                </ul>
              </Show>
              <Nights detail={d()} />
            </Show>
            <Show when={props.history}>
              <h3>更新の履歴</h3>
              <History changes={d().history} />
            </Show>
          </>
        )}
      </Show>
    </div>
  );
}

function Loaded(props: { id: string; layer: string; onClose: () => void }) {
  const detail = createAsync(() => memoryQuery(props.id, ""));
  return (
    <Show when={detail()} fallback={<Loading />}>
      {(d) => (
        <Switch fallback={<MemoryDetail detail={d()} history onGone={props.onClose} />}>
          <Match when={d().layer !== props.layer && d().item}>
            {(moved) => (
              <EmptyState
                action={
                  <LinkButton small href={itemHref(moved())}>
                    開く
                  </LinkButton>
                }
              >
                ほかの層に移った
              </EmptyState>
            )}
          </Match>
        </Switch>
      )}
    </Show>
  );
}

/** An item of a one-line layer, over the list it is picked from. */
export function MemorySheet(props: { id: string; layer: string; onClose: () => void }) {
  // What it showed stays while it closes.
  const shown = createMemo<string>((before) => props.id || before, "");
  return (
    <Dialog.Root open={!!props.id} onOpenChange={(d) => !d.open && props.onClose()} lazyMount unmountOnExit>
      <SideSheet name="note" title={`${LAYER_NAMES[props.layer] ?? "記憶"}の詳細`}>
        <Show when={shown()} keyed>
          {(id) => (
            <ErrorBoundary fallback={(error) => <ErrorState error={error} />}>
              <Suspense fallback={<Loading />}>
                <Loaded id={id} layer={props.layer} onClose={props.onClose} />
              </Suspense>
            </ErrorBoundary>
          )}
        </Show>
      </SideSheet>
    </Dialog.Root>
  );
}
