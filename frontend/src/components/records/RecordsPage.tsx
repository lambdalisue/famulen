import { A, createAsync, useNavigate } from "@solidjs/router";
import { createEffect, createMemo, createSignal, For, type JSX, on, onMount, Show } from "solid-js";
import { Button, Checkbox, EmptyState, IconButton, Keyed, List, Loading, PageHeader, SearchBox, Segmented, SegmentedLinks, Select } from "~/components/ui";
import * as icons from "~/lib/icons";
import { useParam, withParams } from "~/lib/params";
import { choiceOf, kindsQuery, nameOf, type Rec, recordsQuery, valueOf } from "~/lib/records";
import { keep, kept } from "~/lib/storage";
import { useOwner } from "~/lib/owner";
import { byName } from "~/lib/words";
import { workgroup } from "~/lib/workgroup";
import AddRecord from "./AddRecord";
import { createNarrowing } from "./narrowing";
import RecordsFrame from "./RecordsFrame";
import { RecordRow, type RowParts } from "./Row";

/** How many of a tab's records the view of all tabs shows. */
const GLIMPSE = 5;
/** How many more rows a list draws each time it is asked to. */
const PAGE = 100;
/** How many tabs stand side by side; the rest are picked from a list. */
const TABS = 8;

export type Group = { name: string; records: Rec[]; folded?: boolean };

/** How a kind is added: from a title and a few fields, or in a dialog of its own. */
type Add = { label: string } & ({ titleLabel: string; fields: string[]; preset?: { [name: string]: string } } | { onAdd: () => void });

/** Bring the tab picked into view along its strip, without moving the page. */
function showCurrent(strip: HTMLElement) {
  const on = strip.querySelector<HTMLElement>("[aria-current]");
  if (!on) return;
  const [s, o] = [strip.getBoundingClientRect(), on.getBoundingClientRect()];
  if (o.left < s.left) strip.scrollLeft -= s.left - o.left;
  else if (o.right > s.right) strip.scrollLeft += o.right - s.right;
}

/** A list or cards of one kind, narrowed by words, labels and choices, with what is picked beside it. */
export default function RecordsPage(props: {
  kind: string;
  /** The page's own path, with what it keeps in its address. */
  base: string;
  title: string;
  hint?: string;
  /** Choice fields to narrow by. */
  narrow: string[];
  /** Left out unless asked for: what is over. */
  over?: (r: Rec) => boolean;
  overLabel?: string;
  groups?: (records: Rec[]) => Group[];
  /** Each value of `field` has a tab of its own, as `of` names it, kept under `param` in the address. */
  tabs?: { label: string; param: string; field: string; of: (r: Rec) => string };
  row: (r: () => Rec) => RowParts;
  /** Can be looked at as cards too. */
  cards?: boolean;
  /** `back` leads to the list it was opened from. */
  detail: (id: string, back: () => string) => JSX.Element;
  /** What switches between views of the page. */
  tools?: JSX.Element;
  add?: Add;
  empty: string;
  /** What stands above the list, and below it. */
  head?: JSX.Element;
  foot?: JSX.Element;
}) {
  const id = useParam("id");
  const owner = useOwner();
  const all = createAsync(() => recordsQuery(props.kind, workgroup()));
  const kinds = createAsync(() => kindsQuery());
  const n = createNarrowing({ all: () => all() ?? [], over: (r) => props.over?.(r) ?? false });
  const [look, setLook] = createSignal("list");
  const [adding, setAdding] = createSignal(false);
  const key = () => `famulen.look.${props.kind}`;
  onMount(() => {
    const was = kept(key());
    if (was === "card" || was === "list") setLook(was);
  });
  const chooseLook = (v: string) => {
    setLook(v);
    keep(key(), v);
  };
  const add = () => {
    if (props.add && "onAdd" in props.add) props.add.onAdd();
    else setAdding(true);
  };

  const choices = (field: string): string[] => {
    const type = kinds()
      ?.find((k) => k.name === props.kind)
      ?.fields.find((f) => f.name === field);
    if (type?.choices.length) return type.choices;
    return [...new Set((all() ?? []).map((r) => valueOf(r, field)).filter((v) => v))].sort();
  };
  const navigate = useNavigate();
  const asked = useParam(props.tabs?.param ?? "");
  // Narrowing changes the counts but not which tabs there are or their order.
  const tabs = createMemo<{ name: string; count: number }[]>(() => {
    const of = props.tabs?.of;
    if (!of) return [];
    const found = new Set(n.found());
    const by = new Map<string, { name: string; count: number; size: number }>();
    for (const r of all() ?? []) {
      if (!found.has(r) && props.over?.(r)) continue;
      const t = by.get(of(r)) ?? { name: of(r), count: 0, size: 0 };
      by.set(t.name, { ...t, count: t.count + (found.has(r) ? 1 : 0), size: t.size + 1 });
    }
    return by.size > 1 ? [...by.values()].sort((a, b) => b.size - a.size || byName(a.name, b.name)) : [];
  });
  // Remembered, as each reading brings the tabs anew and the list is put at its top only by another tab.
  const tab = createMemo(() => (tabs().some((t) => t.name === asked()) ? asked() : ""));
  const tabHref = (name: string) => (props.tabs ? withParams(props.base, { [props.tabs.param]: name }) : props.base);
  const base = () => tabHref(tab());
  const shown = createMemo(() => (tab() ? n.found().filter((r) => props.tabs?.of(r) === tab()) : n.found()));
  const groups = createMemo<Group[]>(() => props.groups?.(shown()) ?? [{ name: "", records: shown() }]);
  const order = createMemo(() => groups().flatMap((g) => g.records));
  /** What is added from a tab belongs to it; the tab of those without a value gives none. */
  const preset = () => {
    const field = props.tabs?.field;
    return field && tab() && (all() ?? []).some((r) => valueOf(r, field) === tab()) ? { [field]: tab() } : {};
  };
  // Another tab is another list, so it starts at its top, wherever the last one was left.
  createEffect(
    on(
      tab,
      () => document.querySelectorAll(".split-left, #pane").forEach((el) => el.scrollTo(0, 0)),
      { defer: true },
    ),
  );

  const Tabs = (p: { class?: string }) => (
    <Show when={tabs().length > 0 && props.tabs}>
      {(t) => (
        <div
          class={`records-tabs ${p.class ?? ""}`}
          data-scrolls-x
          ref={(el) => createEffect(on(tab, () => queueMicrotask(() => showCurrent(el))))}
        >
          <SegmentedLinks
            label={t().label}
            items={[
              [tabHref(""), `すべて ${n.found().length}`, tab() === ""],
              ...tabs()
                .slice(0, TABS)
                .map((x): [string, string, boolean] => [tabHref(x.name), `${x.name} ${x.count}`, x.name === tab()]),
            ]}
          />
          <Show when={tabs().length > TABS}>
            <Select
              aria-label={`ほかの${t().label}`}
              value={tabs().findIndex((x) => x.name === tab()) >= TABS ? tab() : ""}
              onChange={(e) => navigate(tabHref(e.currentTarget.value), { scroll: false })}
            >
              <option value="">ほか</option>
              <For each={tabs().slice(TABS)}>
                {(x) => (
                  <option value={x.name} selected={x.name === tab()}>
                    {x.name} {x.count}
                  </option>
                )}
              </For>
            </Select>
          </Show>
        </div>
      )}
    </Show>
  );

  /** The rows drawn so far; the one picked is always among them. */
  const Some = (p: { records: Rec[]; first: number; more: (left: number, draw: () => void) => JSX.Element }) => {
    const [drawn, setDrawn] = createSignal(p.first);
    const end = () => Math.max(drawn(), p.records.findIndex((r) => r.id === id()) + 1);
    // Once drawn, a row stays when the detail is closed.
    createEffect(() => setDrawn(end()));
    return (
      <>
        <Rows records={p.records.slice(0, end())} />
        <Show when={p.records.length > end()}>{p.more(p.records.length - end(), () => setDrawn(end() + PAGE))}</Show>
      </>
    );
  };
  const Whole = (p: { records: Rec[] }) => (
    <Some
      records={p.records}
      first={PAGE}
      more={(left, draw) => (
        <Button small class="records-more" onClick={draw}>
          あと {left} 件
        </Button>
      )}
    />
  );
  const Glimpse = (p: { records: Rec[]; name: string }) => (
    <Some
      records={p.records}
      first={GLIMPSE}
      more={() => (
        <A class="records-more" href={tabHref(p.name)} noScroll>
          {p.name} {p.records.length} 件を見る
        </A>
      )}
    />
  );

  const Rows = (p: { records: Rec[] }) => (
    <List class={look() === "card" ? "rec-cards" : "rec-rows"}>
      <Keyed each={p.records} by={(r) => r.id}>
        {(r) => (
          <RecordRow
            record={r()}
            href={withParams(base(), { id: r().id })}
            current={r().id === id()}
            over={props.over?.(r())}
            card={look() === "card"}
            parts={props.row(r)}
          />
        )}
      </Keyed>
    </List>
  );

  return (
    <>
      <RecordsFrame
        name={props.kind}
        base={base()}
        title={props.title}
        id={id()}
        order={order()}
        detail={(id) => props.detail(id, base)}
      >
        <PageHeader
          title={props.title}
          tags={<span class="sub">{shown().length} 件</span>}
          actions={
            <>
              <span class="d-only">{props.tools}</span>
              <Show when={props.cards}>
                <Segmented
                  label="並べ方"
                  items={[
                    ["list", "リスト"],
                    ["card", "カード"],
                  ]}
                  value={look()}
                  onChoose={chooseLook}
                />
              </Show>
              <Show when={props.add}>
                {(a) => (
                  <Button variant="primary" icon={icons.plus} class="d-only" onClick={add}>
                    {a().label}
                  </Button>
                )}
              </Show>
            </>
          }
        />
        <div class="records-filter">
          <SearchBox
            label={`${props.title}を探す`}
            placeholder="探す"
            value={n.words()}
            onInput={(e) => n.setWords(e.currentTarget.value)}
          />
          <div class="records-choices" data-scrolls-x>
            <For each={props.narrow}>
              {(field) => (
                <Select
                  aria-label={`${nameOf(field)}で絞る`}
                  value={n.chosen()[field] ?? ""}
                  onChange={(e) => n.choose(field, e.currentTarget.value)}
                >
                  <option value="">{nameOf(field)}：すべて</option>
                  <For each={choices(field)}>{(c) => <option value={c}>{choiceOf(c, owner.aide())}</option>}</For>
                </Select>
              )}
            </For>
            <Show when={props.over}>
              <Checkbox checked={n.withOver()} onChange={(e) => n.setWithOver(e.currentTarget.checked)}>
                {props.overLabel}
              </Checkbox>
            </Show>
            <Show when={n.anyDoubted()}>
              <Checkbox checked={n.onlyDoubted()} onChange={(e) => n.setOnlyDoubted(e.currentTarget.checked)}>
                古いかもしれないものだけ
              </Checkbox>
            </Show>
          </div>
        </div>
        <Show when={n.allLabels().length > 0}>
          <div class="chips-line" role="group" aria-label="ラベルで絞る">
            <For each={n.allLabels()}>
              {(l) => (
                <button type="button" class="tag label" aria-pressed={n.labels().includes(l)} onClick={() => n.toggleLabel(l)}>
                  {l}
                </button>
              )}
            </For>
          </div>
        </Show>
        {props.head}
        <Tabs class="d-only" />
        <Show when={all()} fallback={<Loading />}>
          <Show
            when={shown().length > 0}
            fallback={
              <Show when={n.any()} fallback={<EmptyState>{props.empty}</EmptyState>}>
                <EmptyState
                  action={
                    <Button small onClick={() => n.clear()}>
                      絞り込みをやめる
                    </Button>
                  }
                >
                  当てはまるものがない
                </EmptyState>
              </Show>
            }
          >
            <Keyed each={groups().filter((g) => g.records.length > 0)} by={(g) => `${tab()}/${g.name}`}>
              {(g) => (
                <Show when={g().name} fallback={<Whole records={g().records} />}>
                  <details class="rec-group" open={!g().folded || !!n.words().trim()}>
                    <summary>
                      <h2>{g().name}</h2>
                      <span class="sub">{g().records.length}</span>
                    </summary>
                    <Show when={tabs().length > 0} fallback={<Whole records={g().records} />}>
                      <Glimpse records={g().records} name={g().name} />
                    </Show>
                  </details>
                </Show>
              )}
            </Keyed>
          </Show>
        </Show>
        {props.foot}
        <Show when={props.hint}>
          <p class="sub">{props.hint}</p>
        </Show>
      </RecordsFrame>
      <Show when={props.add && "titleLabel" in props.add ? props.add : undefined}>
        {(a) => <AddRecord kind={props.kind} base={base()} {...a()} preset={{ ...a().preset, ...preset() }} open={adding()} onOpenChange={setAdding} />}
      </Show>
      <Show when={!id()}>
        <div class="m-float">
          <div class="m-toggles">
            {props.tools}
            <Tabs />
          </div>
          <Show when={props.add}>
            {(a) => <IconButton plain class="m-fab" size={28} icon={icons.plus} label={a().label} onClick={add} />}
          </Show>
        </div>
      </Show>
    </>
  );
}
