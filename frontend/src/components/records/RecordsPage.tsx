import { A, createAsync, useNavigate } from "@solidjs/router";
import { createMemo, createEffect, createSignal, For, type JSX, on, onMount, Show } from "solid-js";
import Icon from "~/components/Icon";
import Modal from "~/components/Modal";
import Segs from "~/components/Segs";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import * as icons from "~/lib/icons";
import { choiceOf, kindsQuery, matches, nameOf, newWorkgroups, type Rec, recordsQuery, valueOf } from "~/lib/records";
import { useParam, withParams } from "~/lib/params";
import { keep, kept } from "~/lib/storage";
import { workgroup } from "~/lib/workgroup";
import type { Typed } from "./FieldEdit";
import FieldEdit from "./FieldEdit";
import "./records.css";

export type Group = { name: string; records: Rec[]; folded?: boolean };

/** Ask for a title and the few fields that matter at first. */
function AddRecord(props: { kind: string; label: string; titleLabel: string; fields: string[]; preset?: { [name: string]: string }; base: string; open: boolean; onOpenChange: (open: boolean) => void }) {
  const kinds = createAsync(() => kindsQuery());
  const navigate = useNavigate();
  const [given, setGiven] = createSignal<{ [name: string]: Typed }>({});
  const types = () => props.fields.map((n) => kinds()?.find((k) => k.name === props.kind)?.fields.find((f) => f.name === n)).filter((t) => t !== undefined);
  const add = async (e: SubmitEvent & { currentTarget: HTMLFormElement }) => {
    e.preventDefault();
    const title = String(new FormData(e.currentTarget).get("title") ?? "").trim();
    if (!title) return;
    const made = await act(
      () =>
        api.records.saveRecord({
          kind: props.kind,
          title,
          workgroups: newWorkgroups(workgroup()),
          fields: [
            ...Object.entries(props.preset ?? {}).map(([name, value]) => ({ name, value })),
            ...Object.entries(given()).map(([name, v]) => ({ name, ...v })),
          ],
        }),
      "足した",
    );
    if (!made) return;
    setGiven({});
    props.onOpenChange(false);
    navigate(withParams(props.base, { id: made.id }));
  };
  return (
    <Modal open={props.open} onOpenChange={props.onOpenChange} title={props.label}>
      <form class="dialog-form" onSubmit={add}>
        <label class="dialog-field">
          <span>{props.titleLabel}</span>
          <input class="input" name="title" required autofocus />
        </label>
        <For each={types()}>
          {(t) => (
            <div class="dialog-field">
              <span>{nameOf(t.name)}</span>
              <FieldEdit
                type={t}
                label={nameOf(t.name)}
                field={given()[t.name] ? ({ name: t.name, ...given()[t.name] } as never) : undefined}
                onSave={(v) => setGiven({ ...given(), [t.name]: v })}
              />
            </div>
          )}
        </For>
        <div class="dialog-actions">
          <button type="button" class="btn" onClick={() => props.onOpenChange(false)}>
            やめる
          </button>
          <button type="submit" class="btn primary">
            足す
          </button>
        </div>
      </form>
    </Modal>
  );
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
  row: (r: Rec) => JSX.Element;
  card?: (r: Rec) => JSX.Element;
  detail: (id: string) => JSX.Element;
  tools?: JSX.Element;
  add?: { label: string; titleLabel: string; fields: string[]; preset?: { [name: string]: string } };
  empty: string;
  /** What stands below the list. */
  foot?: JSX.Element;
}) {
  const navigate = useNavigate();
  const id = useParam("id");
  const all = createAsync(() => recordsQuery(props.kind, workgroup()));
  const kinds = createAsync(() => kindsQuery());
  const [words, setWords] = createSignal("");
  const [labels, setLabels] = createSignal<string[]>([]);
  const [chosen, setChosen] = createSignal<{ [field: string]: string }>({});
  const [withOver, setWithOver] = createSignal(false);
  const [onlyDoubted, setOnlyDoubted] = createSignal(false);
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
  const open = (r: Rec) => {
    left = document.getElementById("pane")?.scrollTop ?? 0;
    navigate(withParams(props.base, { id: r.id }), { scroll: false });
  };

  const allLabels = createMemo(() => [...new Set((all() ?? []).flatMap((r) => r.labels))].sort());
  const choices = (field: string): string[] => {
    const type = kinds()
      ?.find((k) => k.name === props.kind)
      ?.fields.find((f) => f.name === field);
    if (type?.choices.length) return type.choices;
    return [...new Set((all() ?? []).map((r) => valueOf(r, field)).filter((v) => v))].sort();
  };
  const found = createMemo(() =>
    (all() ?? []).filter(
      (r) =>
        (withOver() || !props.over?.(r) || Object.values(chosen()).some((v) => v)) &&
        (!onlyDoubted() || !!r.obsolete) &&
        (!words().trim() || matches(r, words().trim())) &&
        labels().every((l) => r.labels.includes(l)) &&
        Object.entries(chosen()).every(([f, v]) => !v || valueOf(r, f) === v),
    ),
  );
  const anyDoubted = createMemo(() => onlyDoubted() || (all() ?? []).some((r) => r.obsolete));
  const groups = createMemo<Group[]>(() => props.groups?.(found()) ?? [{ name: "", records: found() }]);

  const order = createMemo(() => groups().flatMap((g) => g.records));
  const place = () => order().findIndex((r) => r.id === id());
  const hrefTo = (r: Rec) => withParams(props.base, { id: r.id });
  const Step = (p: { to: Rec | undefined; label: string; icon: string }) => (
    <Show
      when={p.to}
      fallback={
        <button type="button" class="btn small" disabled aria-label={p.label}>
          <Icon svg={p.icon} size={14} />
        </button>
      }
    >
      {(r) => (
        <A class="btn small" href={hrefTo(r())} aria-label={p.label} title={r().title}>
          <Icon svg={p.icon} size={14} />
        </A>
      )}
    </Show>
  );

  // The list stays as it was left: what narrows it is kept, and so is how
  // far down it was.
  const pane = () => document.getElementById("pane");
  let left = 0;
  createEffect(
    on(id, (now, before) => {
      if (now !== before) queueMicrotask(() => pane()?.scrollTo(0, now ? 0 : left));
    }),
  );

  const one = (r: Rec) => (
    <div
      class={look() === "card" ? "rec-card" : "rec-row"}
      classList={{ on: r.id === id(), over: props.over?.(r), doubted: !!r.obsolete }}
      data-record={r.id}
      onClick={(e) => {
        const on = e.target instanceof Element ? e.target.closest("a,button,input,select,label") : null;
        if (!on) open(r);
      }}
    >
      {look() === "card" ? (props.card ?? props.row)(r) : props.row(r)}
      <Show when={r.obsolete}>
        <span class="tag warn" title={r.obsoleteReason}>
          古いかも
        </span>
      </Show>
    </div>
  );

  return (
    <div class="records" classList={{ picked: !!id() }}>
      <div class="records-list">
        <header class="records-head">
          <h1>{props.title}</h1>
          <span class="sub">{found().length} 件</span>
          <span class="grow" />
          {props.tools}
          <Show when={props.card}>
            <span class="records-look">
              <Segs
                label="並べ方"
                items={[
                  ["list", "リスト"],
                  ["card", "カード"],
                ]}
                value={look()}
                onChoose={chooseLook}
              />
            </span>
          </Show>
          <Show when={props.add}>
            {(a) => (
              <button type="button" class="btn primary d-only" onClick={() => setAdding(true)}>
                <Icon svg={icons.plus} size={14} />
                {a().label}
              </button>
            )}
          </Show>
        </header>
        <div class="records-filter">
          <label class="service-search grow">
            <Icon svg={icons.search} size={15} />
            <input class="input" type="search" aria-label={`${props.title}を探す`} placeholder="探す" value={words()} onInput={(e) => setWords(e.currentTarget.value)} />
          </label>
          <For each={props.narrow}>
            {(field) => (
              <select
                class="input"
                aria-label={`${nameOf(field)}で絞る`}
                value={chosen()[field] ?? ""}
                onChange={(e) => setChosen({ ...chosen(), [field]: e.currentTarget.value })}
              >
                <option value="">{nameOf(field)}：すべて</option>
                <For each={choices(field)}>{(c) => <option value={c}>{choiceOf(c)}</option>}</For>
              </select>
            )}
          </For>
          <Show when={props.over}>
            <label class="records-over">
              <input type="checkbox" checked={withOver()} onChange={(e) => setWithOver(e.currentTarget.checked)} />
              {props.overLabel}
            </label>
          </Show>
          <Show when={anyDoubted()}>
            <label class="records-over">
              <input type="checkbox" checked={onlyDoubted()} onChange={(e) => setOnlyDoubted(e.currentTarget.checked)} />
              古いかもしれないものだけ
            </label>
          </Show>
        </div>
        <Show when={allLabels().length > 0}>
          <div class="chips-line" role="group" aria-label="ラベルで絞る">
            <For each={allLabels()}>
              {(l) => (
                <button
                  type="button"
                  class="tag label"
                  aria-pressed={labels().includes(l)}
                  onClick={() => setLabels(labels().includes(l) ? labels().filter((x) => x !== l) : [...labels(), l])}
                >
                  {l}
                </button>
              )}
            </For>
          </div>
        </Show>
        <Show when={all()} fallback={<p class="empty">読み込んでいる</p>}>
          <Show when={found().length > 0} fallback={<p class="empty">{(all() ?? []).length === 0 ? props.empty : "当てはまるものがない"}</p>}>
            <For each={groups().filter((g) => g.records.length > 0)}>
              {(g) => {
                const body = () => <div class={look() === "card" ? "rec-cards" : "rec-rows"}><For each={g.records}>{one}</For></div>;
                return (
                  <Show when={g.name} fallback={body()}>
                    <details class="rec-group" open={!g.folded}>
                      <summary>
                        <h2>{g.name}</h2>
                        <span class="sub">{g.records.length}</span>
                      </summary>
                      {body()}
                    </details>
                  </Show>
                );
              }}
            </For>
          </Show>
        </Show>
        {props.foot}
        <Show when={props.hint}>
          <p class="sub">{props.hint}</p>
        </Show>
      </div>
      <Show when={id()} keyed>
        {(picked) => (
          <div class="records-detail">
            <nav class="records-nav d-only" aria-label="一覧との行き来">
              <A class="btn small" href={props.base}>
                <Icon svg={icons.chevronLeft} size={14} />
                {props.title}の一覧
              </A>
              <span class="grow" />
              <Show when={place() >= 0}>
                <span class="sub">
                  {place() + 1} / {order().length}
                </span>
                <Step to={order()[place() - 1]} label="前の項目" icon={icons.arrowUp} />
                <Step to={order()[place() + 1]} label="次の項目" icon={icons.arrowDown} />
              </Show>
            </nav>
            {props.detail(picked)}
          </div>
        )}
      </Show>
      <Show when={props.add}>
        {(a) => (
          <>
            <AddRecord kind={props.kind} base={props.base} {...a()} open={adding()} onOpenChange={setAdding} />
            <div class="m-float records-fab">
              <span />
              <button type="button" class="m-fab" aria-label={a().label} onClick={() => setAdding(true)}>
                <Icon svg={icons.plus} size={28} />
              </button>
            </div>
          </>
        )}
      </Show>
    </div>
  );
}
