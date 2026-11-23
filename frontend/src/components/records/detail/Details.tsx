import { Index } from "solid-js";
import { Button, IconButton, TextField, whenConfirmed } from "~/components/ui";
import * as icons from "~/lib/icons";
import { type FieldLike, plainField } from "~/lib/records";

/** The owner's own fields. */
export default function Details(props: { fields: FieldLike[]; onSave: (change: (fields: FieldLike[]) => FieldLike[]) => void }) {
  const put = (at: number, f: FieldLike | undefined) => props.onSave((fields) => fields.flatMap((x, n) => (n === at ? (f ? [f] : []) : [x])));
  return (
    <section class="rec-section">
      <h3>ほかの項目</h3>
      <div class="rec-details">
        <Index each={props.fields}>
          {(f, at) => (
            <div class="rec-detail-row">
              <TextField aria-label="項目の名前" value={f().name} onChange={(e) => put(at, { ...plainField(f()), name: e.currentTarget.value.trim() || f().name })} />
              <TextField class="grow" aria-label={`${f().name}の値`} value={f().value} onChange={(e) => put(at, { name: f().name, value: e.currentTarget.value, values: [] })} />
              <IconButton
                plain
                variant="danger"
                icon={icons.trash}
                label={`${f().name}を外す`}
                onClick={() => void whenConfirmed(`「${f().name}」を外す`, "外す", async () => put(at, undefined), f().value)}
              />
            </div>
          )}
        </Index>
        <form
          class="rec-detail-row"
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
          <TextField name="name" placeholder="項目の名前" aria-label="足す項目の名前" />
          <TextField class="grow" name="value" placeholder="値" aria-label="足す項目の値" />
          <Button type="submit" small>
            足す
          </Button>
        </form>
      </div>
    </section>
  );
}
