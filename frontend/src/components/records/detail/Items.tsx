import { createSignal, For, Index, Show } from "solid-js";
import { Button, Form, IconButton, TextField, whenConfirmed } from "~/components/ui";
import type { SectionType } from "~/gen/famulen/v1/records_pb";
import * as icons from "~/lib/icons";
import { type FieldLike, type FieldSpec, type ItemLike, linkOf, nameOf, same, withField } from "~/lib/records";
import FieldEdit, { FieldView } from "../FieldEdit";

/** What an item is called: its first field, which is the item's own text. */
const textField = (name: string, text: string): FieldLike => ({ name, value: text, values: text ? [text] : [] });

/** A field the section does not name, kept as it was written. */
const written = (name: string): FieldSpec => ({ name, type: "text", choices: [], of: "" });

function Item(props: {
  type: SectionType;
  item: ItemLike;
  aide: string;
  onChange: (f: (i: ItemLike) => ItemLike) => void;
  onRemove: () => void;
}) {
  const [editing, setEditing] = createSignal(false);
  const first = () => props.type.fields[0];
  const rest = () => props.type.fields.slice(1);
  const title = () => linkOf(props.item.text)?.title || props.item.text;
  return (
    <div class="rec-item">
      <div class="rec-item-head">
        <b class="grow">
          <FieldView type={first()} field={{ name: first().name, value: props.item.text, values: [] }} aide={props.aide} />
        </b>
        <IconButton
          plain
          icon={editing() ? icons.check : icons.pencil}
          label={editing() ? "直し終える" : "直す"}
          title={`「${title()}」を${editing() ? "直し終える" : "直す"}`}
          onClick={() => setEditing(!editing())}
        />
        <IconButton
          plain
          variant="danger"
          icon={icons.trash}
          label="外す"
          title={`「${title()}」を外す`}
          onClick={() => whenConfirmed(`${nameOf(props.type.name)}から「${title()}」を外す`, "外す", async () => props.onRemove())}
        />
      </div>
      <Show
        when={editing()}
        fallback={
          <Show when={props.item.fields.length > 0}>
            <dl class="rec-fields inline">
              <For each={props.item.fields}>
                {(f) => (
                  <div class="rec-field">
                    <dt>{nameOf(f.name)}</dt>
                    <dd>
                      <FieldView type={rest().find((t) => same(t.name, f.name)) ?? written(f.name)} field={f} aide={props.aide} />
                    </dd>
                  </div>
                )}
              </For>
            </dl>
          </Show>
        }
      >
        <dl class="rec-fields rows">
          <div class="rec-field">
            <dt>{nameOf(first().name)}</dt>
            <dd>
              <FieldEdit
                type={first()}
                label={nameOf(first().name)}
                field={textField(first().name, props.item.text)}
                onSave={(v) => {
                  const text = v.value || v.values[0];
                  if (text) props.onChange((i) => ({ ...i, text }));
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
                    field={props.item.fields.find((f) => same(f.name, t.name))}
                    onSave={(v) => props.onChange((i) => ({ ...i, fields: withField(i.fields, t.name, v) }))}
                  />
                </dd>
              </div>
            )}
          </For>
        </dl>
      </Show>
    </div>
  );
}

/** What can be put in without writing it: the owner, or the run before. */
const READY: { [section: string]: { text: string; label: string } } = {
  Relations: { text: "me", label: "あなたとの関係" },
  "Skip if": { text: "previous unfinished", label: "前の回が済むまで" },
};

/** The kind of record an item of a section may name instead of writing it. */
const NAMES: { [field: string]: { of: string; label: string } } = {
  To: { of: "parties", label: "相手" },
  Who: { of: "parties", label: "相手" },
  When: { of: "tasks", label: "タスク" },
};

function NewItem(props: { type: SectionType; onAdd: (i: ItemLike) => void }) {
  const first = () => props.type.fields[0];
  const label = () => `${nameOf(props.type.name)}を足す`;
  const add = (text: string) => {
    if (text.trim()) props.onAdd({ text: text.trim(), fields: [] });
  };
  return (
    <Show
      when={first().type !== "link"}
      fallback={
        <FieldEdit type={first()} label={nameOf(props.type.name)} field={undefined} onSave={(v) => add(v.value || v.values[0] || "")} />
      }
    >
      <Form
        class="rec-new"
        onSubmit={(e) => {
          e.preventDefault();
          const box = e.currentTarget.querySelector("input");
          if (!box) return;
          add(box.value);
          box.value = "";
        }}
      >
        <TextField class="grow" placeholder={label()} aria-label={label()} />
        <Show when={NAMES[first().name]}>
          {(n) => (
            <FieldEdit
              type={{ name: first().name, choices: [], type: "link", of: n().of }}
              label={n().label}
              field={undefined}
              onSave={(v) => add(v.value)}
            />
          )}
        </Show>
        <Show when={READY[props.type.name]}>
          {(r) => (
            <Button small onClick={() => add(r().text)}>
              {r().label}
            </Button>
          )}
        </Show>
        <Button type="submit" small>
          足す
        </Button>
      </Form>
    </Show>
  );
}

/** A section of items: each a card with its own fields. */
export default function Items(props: {
  type: SectionType;
  items: ItemLike[];
  aide: string;
  onSave: (change: (items: ItemLike[]) => ItemLike[]) => void;
}) {
  return (
    <div class="rec-items">
      <Index each={props.items}>
        {(item, at) => (
          <Item
            type={props.type}
            item={item()}
            aide={props.aide}
            onChange={(f) => props.onSave((items) => items.map((i, n) => (n === at ? f(i) : i)))}
            onRemove={() => props.onSave((items) => items.filter((_, n) => n !== at))}
          />
        )}
      </Index>
      <NewItem type={props.type} onAdd={(i) => props.onSave((items) => [...items, i])} />
    </div>
  );
}
