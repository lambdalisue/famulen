import { createAsync, useNavigate } from "@solidjs/router";
import { createSignal, For } from "solid-js";
import { Dialog, DialogActions, Field, Form, TextField } from "~/components/ui";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { withParams } from "~/lib/params";
import { type FieldValue, kindsQuery, nameOf, newWorkgroups } from "~/lib/records";
import { workgroup } from "~/lib/workgroup";
import FieldEdit from "./FieldEdit";

/** Ask for a title and the few fields that matter at first. */
export default function AddRecord(props: {
  kind: string;
  label: string;
  titleLabel: string;
  fields: string[];
  preset?: { [name: string]: string };
  base: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const kinds = createAsync(() => kindsQuery());
  const navigate = useNavigate();
  const [given, setGiven] = createSignal<{ [name: string]: FieldValue }>({});
  const preset = () =>
    Object.fromEntries(Object.entries(props.preset ?? {}).map(([name, value]): [string, FieldValue] => [name, { value, values: [] }]));
  const filled = (): { [name: string]: FieldValue } => ({ ...preset(), ...given() });
  const types = () =>
    props.fields
      .map((n) =>
        kinds()
          ?.find((k) => k.name === props.kind)
          ?.fields.find((f) => f.name === n),
      )
      .filter((t) => t !== undefined);
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
          fields: Object.entries(filled()).map(([name, v]) => ({ name, ...v })),
        }),
      "足した",
    );
    if (!made) return;
    setGiven({});
    props.onOpenChange(false);
    navigate(withParams(props.base, { id: made.id }));
  };
  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange} title={props.label}>
      <Form class="dialog-form" onSubmit={add}>
        <Field label={props.titleLabel}>
          <TextField name="title" required autofocus />
        </Field>
        <For each={types()}>
          {(t) => (
            <Field label={nameOf(t.name)} group>
              <FieldEdit
                type={t}
                label={nameOf(t.name)}
                field={filled()[t.name] ? { name: t.name, ...filled()[t.name] } : undefined}
                onSave={(v) => setGiven({ ...given(), [t.name]: v })}
              />
            </Field>
          )}
        </For>
        <DialogActions submit="足す" />
      </Form>
    </Dialog>
  );
}
