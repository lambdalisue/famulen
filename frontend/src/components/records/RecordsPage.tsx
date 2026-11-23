import { createAsync } from "@solidjs/router";
import { createMemo, createSignal, For, type JSX, onMount, Show } from "solid-js";
import { Button, Checkbox, EmptyState, IconButton, Keyed, List, Loading, PageHeader, SearchBox, Segmented, Select } from "~/components/ui";
import * as icons from "~/lib/icons";
import { useParam, withParams } from "~/lib/params";
import { choiceOf, kindsQuery, nameOf, type Rec, recordsQuery, valueOf } from "~/lib/records";
import { keep, kept } from "~/lib/storage";
import { useOwner } from "~/lib/owner";
import { workgroup } from "~/lib/workgroup";
import AddRecord from "./AddRecord";
import { createNarrowing } from "./narrowing";
import RecordsFrame from "./RecordsFrame";
import { RecordRow, type RowParts } from "./Row";

export type Group = { name: string; records: Rec[]; folded?: boolean };

/** How a kind is added: from a title and a few fields, or in a dialog of its own. */
type Add = { label: string } & ({ titleLabel: string; fields: string[]; preset?: { [name: string]: string } } | { onAdd: () => void });

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
  row: (r: () => Rec) => RowParts;
  /** Can be looked at as cards too. */
  cards?: boolean;
  detail: (id: string) => JSX.Element;
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
  const groups = createMemo<Group[]>(() => props.groups?.(n.found()) ?? [{ name: "", records: n.found() }]);
  const order = createMemo(() => groups().flatMap((g) => g.records));

  const Rows = (p: { records: Rec[] }) => (
    <List class={look() === "card" ? "rec-cards" : "rec-rows"}>
      <Keyed each={p.records} by={(r) => r.id}>
        {(r) => (
          <RecordRow
            record={r()}
            href={withParams(props.base, { id: r().id })}
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
      <RecordsFrame name={props.kind} base={props.base} title={props.title} id={id()} order={order()} detail={props.detail}>
        <PageHeader
          title={props.title}
          tags={<span class="sub">{n.found().length} 件</span>}
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
          <SearchBox label={`${props.title}を探す`} placeholder="探す" value={n.words()} onInput={(e) => n.setWords(e.currentTarget.value)} />
          <div class="records-choices" data-scrolls-x>
            <For each={props.narrow}>
              {(field) => (
                <Select aria-label={`${nameOf(field)}で絞る`} value={n.chosen()[field] ?? ""} onChange={(e) => n.choose(field, e.currentTarget.value)}>
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
        <Show when={all()} fallback={<Loading />}>
          <Show
            when={n.found().length > 0}
            fallback={
              <Show
                when={n.any()}
                fallback={
                  <EmptyState>{props.empty}</EmptyState>
                }
              >
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
            <Keyed each={groups().filter((g) => g.records.length > 0)} by={(g) => g.name}>
              {(g) => (
                <Show when={g().name} fallback={<Rows records={g().records} />}>
                  <details class="rec-group" open={!g().folded}>
                    <summary>
                      <h2>{g().name}</h2>
                      <span class="sub">{g().records.length}</span>
                    </summary>
                    <Rows records={g().records} />
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
        {(a) => <AddRecord kind={props.kind} base={props.base} {...a()} open={adding()} onOpenChange={setAdding} />}
      </Show>
      <Show when={!id()}>
        <div class="m-float">
          <div class="m-toggles">{props.tools}</div>
          <Show when={props.add}>
            {(a) => <IconButton plain class="m-fab" size={28} icon={icons.plus} label={a().label} onClick={add} />}
          </Show>
        </div>
      </Show>
    </>
  );
}
