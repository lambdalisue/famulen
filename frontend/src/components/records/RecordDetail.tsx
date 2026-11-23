import { createAsync, useNavigate } from "@solidjs/router";
import { createEffect, createSignal, For, Index, type JSX, on, Show } from "solid-js";
import Icon from "~/components/Icon";
import Markdown from "~/components/Markdown";
import Segs from "~/components/Segs";
import type { SectionType } from "~/gen/famulen/v1/records_pb";
import { act, say } from "~/lib/act";
import { api } from "~/lib/api";
import { personaQuery, workgroupsQuery } from "~/lib/data";
import * as icons from "~/lib/icons";
import {
  type Field,
  fieldOf,
  type FieldType,
  hrefOf,
  KIND_NAMES,
  kindsQuery,
  nameOf,
  type Rec,
  type RecordItem,
  recordQuery,
} from "~/lib/records";
import { setSelection } from "~/lib/view";
import { A } from "@solidjs/router";
import FieldEdit, { FieldView, type Typed } from "./FieldEdit";

type PlainField = { name: string; value: string; values: string[] };
type PlainItem = { text: string; fields: PlainField[] };

const plain = (f: Field): PlainField => ({ name: f.name, value: f.value, values: f.values });
const plainItem = (i: RecordItem): PlainItem => ({ text: i.text, fields: i.fields.map(plain) });
const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

/** A field shown as read, edited in place once pressed. */
function FieldRow(props: { type: FieldType; field: Field | undefined; aide: string; open?: boolean; onSave: (v: Typed) => void }) {
  const [editing, setEditing] = createSignal(props.open ?? false);
  const label = () => nameOf(props.type.name);
  return (
    <div class="rec-field">
      <dt>{label()}</dt>
      <dd>
        <Show
          when={editing()}
          fallback={
            <>
              <span class="rec-value">
                <FieldView type={props.type} field={props.field} aide={props.aide} />
              </span>
              <button type="button" class="rec-edit" aria-label={`${label()}を直す`} onClick={() => setEditing(true)}>
                <Icon svg={icons.pencil} size={13} />
              </button>
            </>
          }
        >
          <FieldEdit type={props.type} label={label()} field={props.field} onSave={props.onSave} />
          <button type="button" class="rec-edit" aria-label={`${label()}を直し終える`} onClick={() => setEditing(false)}>
            <Icon svg={icons.check} size={13} />
          </button>
        </Show>
      </dd>
    </div>
  );
}

/** A list of fields, with a way to fill in those that are not given yet. */
function Fields(props: { types: FieldType[]; fields: Field[]; aide: string; skip?: string[]; onSave: (name: string, v: Typed) => void }) {
  const [added, setAdded] = createSignal<string[]>([]);
  const shown = () =>
    props.types.filter((t) => !props.skip?.includes(t.name) && (fieldOf(props, t.name) || added().includes(t.name)));
  const missing = () => props.types.filter((t) => !props.skip?.includes(t.name) && !shown().includes(t));
  const unknown = () => props.fields.filter((f) => !props.types.some((t) => same(t.name, f.name)));
  return (
    <>
      <dl class="rec-fields">
        <For each={shown()}>
          {(t) => (
            <FieldRow
              type={t}
              field={fieldOf(props, t.name)}
              aide={props.aide}
              open={added().includes(t.name)}
              onSave={(v) => props.onSave(t.name, v)}
            />
          )}
        </For>
        <For each={unknown()}>
          {(f) => (
            <div class="rec-field">
              <dt>{nameOf(f.name)}</dt>
              <dd>{f.value}</dd>
            </div>
          )}
        </For>
      </dl>
      <Show when={missing().length > 0}>
        <select
          class="rec-add"
          aria-label="項目を足す"
          onChange={(e) => {
            const name = e.currentTarget.value;
            if (name) setAdded([...added(), name]);
            e.currentTarget.value = "";
          }}
        >
          <option value="">＋ 項目を足す</option>
          <For each={missing()}>{(t) => <option value={t.name}>{nameOf(t.name)}</option>}</For>
        </select>
      </Show>
    </>
  );
}

/** A section of items: each a card with its own fields. */
function Items(props: { type: SectionType; items: RecordItem[]; aide: string; onSave: (change: (items: PlainItem[]) => PlainItem[]) => void }) {
  const [editing, setEditing] = createSignal<number | undefined>();
  const first = () => props.type.fields[0];
  const rest = () => props.type.fields.slice(1);
  const change = (at: number, f: (i: PlainItem) => PlainItem) => props.onSave((items) => items.map((i, n) => (n === at ? f(i) : i)));
  return (
    <div class="rec-items">
      <Index each={props.items}>
        {(item, at) => (
          <div class="rec-item">
            <div class="rec-item-head">
              <b class="grow">
                <FieldView type={first()} field={{ name: first().name, value: item().text, values: [] } as unknown as Field} aide={props.aide} />
              </b>
              <button
                type="button"
                class="rec-edit"
                aria-label={editing() === at ? "直し終える" : "直す"}
                onClick={() => setEditing(editing() === at ? undefined : at)}
              >
                <Icon svg={editing() === at ? icons.check : icons.pencil} size={13} />
              </button>
              <button type="button" class="rec-edit" aria-label="外す" onClick={() => props.onSave((items) => items.filter((_, n) => n !== at))}>
                <Icon svg={icons.trash} size={13} />
              </button>
            </div>
            <Show
              when={editing() === at}
              fallback={
                <Show when={item().fields.length > 0}>
                  <dl class="rec-fields inline">
                    <For each={item().fields}>
                      {(f) => (
                        <div class="rec-field">
                          <dt>{nameOf(f.name)}</dt>
                          <dd>
                            <FieldView
                              type={rest().find((t) => same(t.name, f.name)) ?? ({ name: f.name, type: "text", choices: [], of: "" } as unknown as FieldType)}
                              field={f}
                              aide={props.aide}
                            />
                          </dd>
                        </div>
                      )}
                    </For>
                  </dl>
                </Show>
              }
            >
              <dl class="rec-fields">
                <div class="rec-field">
                  <dt>{nameOf(first().name)}</dt>
                  <dd>
                    <FieldEdit
                      type={first()}
                      label={nameOf(first().name)}
                      field={{ name: first().name, value: item().text, values: item().text ? [item().text] : [] } as unknown as Field}
                      onSave={(v) => {
                        const text = v.value || v.values[0];
                        if (text) change(at, (i) => ({ ...i, text }));
                      }}
                    />
                  </dd>
                </div>
                <For each={rest()}>
                  {(t) => (
                    <div class="rec-field">
                      <dt>{nameOf(t.name)}</dt>
                      <dd>
                        <FieldEdit
                          type={t}
                          label={nameOf(t.name)}
                          field={item().fields.find((f) => same(f.name, t.name))}
                          onSave={(v) =>
                            change(at, (i) => {
                              const left = i.fields.filter((f) => !same(f.name, t.name));
                              const empty = !v.value && v.values.length === 0;
                              return { ...i, fields: empty ? left : [...left, { name: t.name, ...v }] };
                            })
                          }
                        />
                      </dd>
                    </div>
                  )}
                </For>
              </dl>
            </Show>
          </div>
        )}
      </Index>
      <NewItem type={props.type} onAdd={(i) => props.onSave((items) => [...items, i])} />
    </div>
  );
}

function NewItem(props: { type: SectionType; onAdd: (i: PlainItem) => void }) {
  const first = () => props.type.fields[0];
  const label = () => `${nameOf(props.type.name)}を足す`;
  const add = (text: string) => {
    if (text.trim()) props.onAdd({ text: text.trim(), fields: [] });
  };
  return (
    <div class="rec-new">
      <Show
        when={first().type !== "link"}
        fallback={<FieldEdit type={first()} label={nameOf(props.type.name)} field={undefined} onSave={(v) => add(v.value || v.values[0] || "")} />}
      >
        <form
          class="row"
          onSubmit={(e) => {
            e.preventDefault();
            const box = e.currentTarget.querySelector("input");
            if (!box) return;
            add(box.value);
            box.value = "";
          }}
        >
          <input class="input grow" placeholder={label()} aria-label={label()} />
          <Show when={first().name === "To" || first().name === "Who" || first().name === "When"}>
            <FieldEdit
              type={{ ...first(), type: "link", of: first().name === "When" ? "tasks" : "parties" }}
              label={first().name === "When" ? "タスク" : "相手"}
              field={undefined}
              onSave={(v) => add(v.value)}
            />
          </Show>
          <Show when={props.type.name === "Relations"}>
            <button type="button" class="btn small" onClick={() => add("me")}>
              あなたとの関係
            </button>
          </Show>
          <Show when={props.type.name === "Skip if"}>
            <button type="button" class="btn small" onClick={() => add("previous unfinished")}>
              前の回が済むまで
            </button>
          </Show>
          <button type="submit" class="btn small">
            足す
          </button>
        </form>
      </Show>
    </div>
  );
}

/** The owner's own fields. */
function Details(props: { fields: Field[]; onSave: (change: (fields: PlainField[]) => PlainField[]) => void }) {
  const put = (at: number, f: PlainField | undefined) => props.onSave((fields) => fields.flatMap((x, n) => (n === at ? (f ? [f] : []) : [x])));
  return (
    <div class="rec-details">
      <Index each={props.fields}>
        {(f, at) => (
          <div class="row">
            <input class="input" aria-label="項目の名前" value={f().name} onChange={(e) => put(at, { ...plain(f()), name: e.currentTarget.value.trim() || f().name })} />
            <input class="input grow" aria-label={`${f().name}の値`} value={f().value} onChange={(e) => put(at, { name: f().name, value: e.currentTarget.value, values: [] })} />
            <button type="button" class="rec-edit" aria-label={`${f().name}を外す`} onClick={() => put(at, undefined)}>
              <Icon svg={icons.trash} size={13} />
            </button>
          </div>
        )}
      </Index>
      <form
        class="row"
        onSubmit={(e) => {
          e.preventDefault();
          const form = e.currentTarget;
          const f = new FormData(form);
          const name = String(f.get("name") ?? "").trim();
          if (!name) return;
          const value = String(f.get("value") ?? "").trim();
          props.onSave((fields) => [...fields, { name, value, values: [] }]);
          form.reset();
        }}
      >
        <input class="input" name="name" placeholder="項目の名前" aria-label="足す項目の名前" />
        <input class="input grow" name="value" placeholder="値" aria-label="足す項目の値" />
        <button type="submit" class="btn small">
          足す
        </button>
      </form>
    </div>
  );
}

/** Free Markdown: read as Arto draws it, written as plain text. */
function Notes(props: { text: string; label: string; empty: string; onSave: (text: string) => Promise<boolean> }) {
  const [draft, setDraft] = createSignal<string | undefined>();
  const [preview, setPreview] = createSignal(false);
  const save = async () => {
    if (await props.onSave(draft() ?? "")) setDraft(undefined);
  };
  return (
    <section class="rec-section">
      <header class="rec-head">
        <h3 class="grow">{props.label}</h3>
        <Show
          when={draft() !== undefined}
          fallback={
            <button type="button" class="btn small" aria-label={`${props.label}を書く`} onClick={() => setDraft(props.text)}>
              <Icon svg={icons.pencil} size={13} />
              書く
            </button>
          }
        >
          <button type="button" class="btn small" onClick={() => setPreview(!preview())}>
            {preview() ? "書く" : "プレビュー"}
          </button>
          <button
            type="button"
            class="btn small"
            onClick={() => {
              if (draft() === props.text || confirm("書きかけを捨てますか")) setDraft(undefined);
            }}
          >
            やめる
          </button>
          <button type="button" class="btn small primary" onClick={save}>
            保存
          </button>
        </Show>
      </header>
      <Show when={draft() === undefined || preview()} fallback={
        <textarea
          class="input rec-notes"
          aria-label={props.label}
          placeholder="Markdown で書ける"
          value={draft()}
          onInput={(e) => setDraft(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void save();
          }}
        />
      }>
        <Show when={(preview() && draft() !== undefined ? draft() : props.text)} fallback={<p class="empty">{props.empty}</p>}>
          {(text) => <Markdown text={text()} />}
        </Show>
      </Show>
    </section>
  );
}

const FROM: { [from: string]: string } = {
  Parent: "子のタスク",
  "Blocked by": "これを待つタスク",
  Related: "関わるもの",
  "Member of": "属する人",
  Relations: "関係",
  Covers: "これを対象にする契約",
  Attendees: "参加する予定",
  Assignee: "任せているタスク",
  Notes: "備考で触れているもの",
  Calendar: "予定",
  "": "触れているもの",
};

function Links(props: { record: Rec }) {
  const groups = () => {
    const by = new Map<string, Rec["backlinks"]>();
    for (const l of props.record.backlinks) {
      const key = FROM[l.from] ?? nameOf(l.from);
      by.set(key, [...(by.get(key) ?? []), l]);
    }
    return [...by];
  };
  return (
    <Show when={groups().length > 0}>
      <section class="rec-section">
        <h3>つながり</h3>
        <For each={groups()}>
          {([name, links]) => (
            <div class="rec-links">
              <span class="sub">{name}</span>
              <span class="chips-line">
                <For each={links}>
                  {(l) => (
                    <Show when={hrefOf(l.kind, l.id)} fallback={<span class="tag">{l.title}</span>}>
                      {(href) => (
                        <A class="tag link" href={href()}>
                          {l.title}
                        </A>
                      )}
                    </Show>
                  )}
                </For>
              </span>
            </div>
          )}
        </For>
      </section>
    </Show>
  );
}

function Log(props: { record: Rec; aide: string; onAdd: (line: string) => Promise<boolean> }) {
  const who = (w: string) => ({ owner: "あなた", secretary: props.aide, famulen: "Famulen" })[w] ?? w;
  const lines = () => [...props.record.log].reverse();
  return (
    <section class="rec-section">
      <h3>経過</h3>
      <form
        class="row"
        onSubmit={(e) => {
          e.preventDefault();
          const box = e.currentTarget.querySelector("input");
          if (!box?.value.trim()) return;
          void props.onAdd(box.value.trim()).then((ok) => {
            if (ok) box.value = "";
          });
        }}
      >
        <input class="input grow" placeholder="あったことを書く" aria-label="経過を足す" />
        <button type="submit" class="btn small">
          足す
        </button>
      </form>
      <ol class="rec-log">
        <For each={lines()}>
          {(l) => (
            <li>
              <span class="sub mono">{l.at}</span>
              <Show when={l.who}>
                <span class="sub">{who(l.who)}</span>
              </Show>
              <span class="rec-log-text">{l.text}</span>
            </li>
          )}
        </For>
      </ol>
    </section>
  );
}

function Labels(props: { record: Rec; onSave: (change: (labels: string[]) => string[]) => void }) {
  return (
    <span class="chips-edit">
      <For each={props.record.labels}>
        {(l) => (
          <span class="tag label removable">
            {l}
            <button type="button" aria-label={`ラベル ${l} を外す`} onClick={() => props.onSave((labels) => labels.filter((x) => x !== l))}>
              <Icon svg={icons.x} size={12} />
            </button>
          </span>
        )}
      </For>
      <input
        class="input label-add"
        placeholder="ラベルを足す"
        aria-label="ラベルを足す"
        onKeyDown={(e) => {
          if (e.key !== "Enter" || e.isComposing) return;
          e.preventDefault();
          const l = e.currentTarget.value.trim();
          if (l) props.onSave((labels) => (labels.includes(l) ? labels : [...labels, l]));
          e.currentTarget.value = "";
        }}
      />
    </span>
  );
}

/** Everything one record says, each part changed where it stands. */
export default function RecordDetail(props: {
  kind: string;
  id: string;
  /** Where to go once it is closed or removed. */
  back: string;
  /** What this kind shows above the rest. */
  top?: (r: Rec) => JSX.Element;
  /** What this kind shows below its fields. */
  extra?: (r: Rec) => JSX.Element;
}) {
  const navigate = useNavigate();
  const record = createAsync(() => recordQuery(props.kind, props.id));
  const kinds = createAsync(() => kindsQuery());
  const workgroups = createAsync(() => workgroupsQuery());
  const persona = createAsync(() => personaQuery());
  const aide = () => persona()?.name || "秘書";
  const kind = () => kinds()?.find((k) => k.name === props.kind);
  const [shown, setShown] = createSignal("view");
  const [addedSections, setAddedSections] = createSignal<string[]>([]);
  createEffect(
    on(
      () => props.id,
      () => {
        setShown("view");
        setAddedSections([]);
      },
    ),
  );
  createEffect(() => {
    const r = record();
    if (r) setSelection(`${KIND_NAMES[r.kind] ?? r.kind}：${r.title}（${r.code}）`);
  });

  type Change = Omit<Parameters<typeof api.records.saveRecord>[0], "$typeName" | "kind" | "id">;
  /**
   * Save one change after another, each worked out from the record as it is
   * by then: a section goes back whole, and what was saved a moment ago, or
   * by the secretary meanwhile, must not be lost with it.
   */
  let last: Promise<unknown> = Promise.resolve();
  const save = (change: Change | ((now: Rec) => Change), done = "保存した") => {
    const { kind, id } = props;
    const turn = last.then(() =>
      act(async () => {
        const what = typeof change === "function" ? change(await api.records.getRecord({ kind, id })) : change;
        return api.records.saveRecord({ kind, id, ...what });
      }, done),
    );
    last = turn;
    return turn;
  };
  const sectionIn = (r: Rec, name: string) => r.sections.find((x) => same(x.name, name));
  const remove = (r: Rec) => {
    if (!confirm(`「${r.title}」を消しますか`)) return;
    void act(() => api.records.deleteRecord({ kind: r.kind, id: r.id }), "消した").then((ok) => {
      if (ok) navigate(props.back);
    });
  };
  const copy = (text: string, what: string) => void navigator.clipboard?.writeText(text).then(() => say(`${what}を写した`));

  return (
    <Show when={record()} fallback={<p class="empty">この記録はもうない。</p>}>
      {(r) => (
        <article class="rec">
          <header class="rec-title">
            <div class="row">
              <button type="button" class="tag mono" title="呼び名を写す" aria-label={`呼び名 ${r().code} を写す`} onClick={() => copy(r().code, "呼び名")}>
                {r().code}
                <Icon svg={icons.copy} size={11} />
              </button>
              <span class="grow" />
              <Segs
                label="見方"
                items={[
                  ["view", "内容"],
                  ["raw", "原文"],
                ]}
                value={shown()}
                onChoose={setShown}
              />
              <button type="button" class="btn small danger" aria-label={`${r().title}を消す`} onClick={() => remove(r())}>
                <Icon svg={icons.trash} size={13} />
              </button>
            </div>
            <input
              class="rec-name"
              aria-label="題名"
              value={r().title}
              onChange={(e) => {
                const title = e.currentTarget.value.trim();
                if (title && title !== r().title) void save({ title });
                else e.currentTarget.value = r().title;
              }}
            />
            <div class="row">
              <Labels record={r()} onSave={(change) => void save((now) => ({ labels: change(now.labels), setLabels: true }))} />
              <span class="chips-line">
                <For each={(workgroups() ?? []).filter((w) => w.id !== "all")}>
                  {(w) => {
                    const on = () => r().workgroups.includes(w.id);
                    return (
                      <button
                        type="button"
                        class="tag wg"
                        aria-pressed={on()}
                        onClick={() =>
                          void save((now) => ({
                            workgroups: now.workgroups.includes(w.id) ? now.workgroups.filter((x) => x !== w.id) : [...now.workgroups, w.id],
                            setWorkgroups: true,
                          }))
                        }
                      >
                        <span class="dot" style={{ background: on() ? w.color : "#CFCCC2" }} />
                        {w.name}
                      </button>
                    );
                  }}
                </For>
              </span>
            </div>
          </header>
          <Show
            when={shown() === "view"}
            fallback={
              <section class="rec-section">
                <header class="rec-head">
                  <span class="sub mono grow">
                    records/{r().kind}/{r().id}.md
                  </span>
                  <button type="button" class="btn small" onClick={() => copy(r().markdown, "原文")}>
                    <Icon svg={icons.copy} size={13} />
                    写す
                  </button>
                </header>
                <pre class="rec-raw">{r().markdown}</pre>
              </section>
            }
          >
            {props.top?.(r())}
            <div class="rec-columns" classList={{ plain: kind()?.free }}>
            <div class="rec-aside">
            <Show when={kind()}>
              {(k) => (
                <>
                  <Show when={k().fields.length > 0 || r().fields.length > 0}>
                    <section class="rec-section">
                      <Fields types={k().fields} fields={r().fields} aide={aide()} onSave={(name, v) => void save({ fields: [{ name, ...v }] })} />
                    </section>
                  </Show>
                  <For each={k().sections.filter((s) => r().sections.some((x) => same(x.name, s.name)) || addedSections().includes(s.name))}>
                    {(s) => {
                      const now = () => r().sections.find((x) => same(x.name, s.name));
                      return (
                        <section class="rec-section">
                          <h3>{nameOf(s.name)}</h3>
                          <Show
                            when={s.many}
                            fallback={
                              <Fields
                                types={s.fields}
                                fields={now()?.fields ?? []}
                                aide={aide()}
                                onSave={(name, v) => {
                                  const empty = !v.value && v.values.length === 0;
                                  void save((latest) => {
                                    const left = (sectionIn(latest, s.name)?.fields ?? []).filter((f) => !same(f.name, name)).map(plain);
                                    return { sections: [{ name: s.name, fields: empty ? left : [...left, { name, ...v }] }] };
                                  });
                                }}
                              />
                            }
                          >
                            <Items type={s} items={now()?.items ?? []} aide={aide()} onSave={(change) =>
                                void save((latest) => ({ sections: [{ name: s.name, items: change((sectionIn(latest, s.name)?.items ?? []).map(plainItem)) }] }))
                              } />
                          </Show>
                        </section>
                      );
                    }}
                  </For>
                  <Show when={k().sections.some((s) => !r().sections.some((x) => same(x.name, s.name)) && !addedSections().includes(s.name))}>
                    <div class="chips-line">
                      <For each={k().sections.filter((s) => !r().sections.some((x) => same(x.name, s.name)) && !addedSections().includes(s.name))}>
                        {(s) => (
                          <button type="button" class="tag add" onClick={() => setAddedSections([...addedSections(), s.name])}>
                            ＋ {nameOf(s.name)}
                          </button>
                        )}
                      </For>
                    </div>
                  </Show>
                </>
              )}
            </Show>
            <Links record={r()} />
            <Show when={!kind()?.free}>
              <section class="rec-section">
                <h3>ほかの項目</h3>
                <Details fields={r().details} onSave={(change) => void save((now) => ({ details: change(now.details.map(plain)), setDetails: true }))} />
              </section>
            </Show>
            </div>
            <div class="rec-body">
            <Notes
              text={r().notes}
              label={kind()?.free ? "本文" : "備考"}
              empty={kind()?.free ? "まだ何も書いていない" : "備考はまだない"}
              onSave={async (notes) => (await save({ notes })) !== undefined}
            />
            {props.extra?.(r())}
            <Show when={!kind()?.free}>
              <Log record={r()} aide={aide()} onAdd={async (line) => (await save({ log: [line] }, "書いた")) !== undefined} />
            </Show>
            </div>
            </div>
            <p class="sub rec-foot">
              <Show when={r().source}>{r().source} · </Show>
              <Show when={r().created}>作った {r().created.slice(0, 16).replace("T", " ")} · </Show>
              <Show when={r().updated}>直した {r().updated.slice(0, 16).replace("T", " ")}</Show>
            </p>
          </Show>
        </article>
      )}
    </Show>
  );
}
