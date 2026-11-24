import { createSignal, For, Show } from "solid-js";
import { IconButton } from "~/components/ui";
import * as icons from "~/lib/icons";
import { type FieldLike, fieldOf, type FieldSpec, type FieldValue, nameOf, same } from "~/lib/records";
import FieldEdit, { FieldView } from "../FieldEdit";
import LinkedText from "../links/LinkedText";

/** A field shown as read, edited in place once pressed. */
function FieldRow(props: { type: FieldSpec; field: FieldLike | undefined; aide: string; open?: boolean; onSave: (v: FieldValue) => void }) {
  const [editing, setEditing] = createSignal(props.open ?? false);
  const label = () => nameOf(props.type.name);
  return (
    <div class="rec-field" classList={{ editing: editing() }}>
      <dt>{label()}</dt>
      <dd>
        <Show
          when={editing()}
          fallback={
            <>
              <span class="rec-value">
                <FieldView type={props.type} field={props.field} aide={props.aide} />
              </span>
              <IconButton plain icon={icons.pencil} label={`${label()}を直す`} onClick={() => setEditing(true)} />
            </>
          }
        >
          <FieldEdit type={props.type} label={label()} field={props.field} onSave={props.onSave} />
          <IconButton plain icon={icons.check} label={`${label()}を直し終える`} onClick={() => setEditing(false)} />
        </Show>
      </dd>
    </div>
  );
}

/** A list of fields, with a way to fill in those that are not given yet. */
export default function Fields(props: {
  types: FieldSpec[];
  fields: FieldLike[];
  aide: string;
  skip?: string[];
  onSave: (name: string, v: FieldValue) => void;
}) {
  const [added, setAdded] = createSignal<string[]>([]);
  const offered = () => props.types.filter((t) => !props.skip?.includes(t.name));
  const shown = () => offered().filter((t) => fieldOf(props, t.name) || added().includes(t.name));
  const missing = () => offered().filter((t) => !shown().includes(t));
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
              <dd>
                <LinkedText text={f.value} />
              </dd>
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
