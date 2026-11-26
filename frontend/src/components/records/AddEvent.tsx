import { createAsync } from "@solidjs/router";
import { createSignal, For, Show } from "solid-js";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { calendarsQuery } from "~/lib/calendar";
import { linkTo, newWorkgroups } from "~/lib/records";
import { workgroup } from "~/lib/workgroup";
import { Checkbox, Dialog, DialogActions, Field, Form, Segmented, Select, TextArea, TextField } from "~/components/ui";

/** Add an event of the notebook, or hold a time. */
export default function AddEvent(props: { day: string; open: boolean; onOpenChange: (open: boolean) => void }) {
  const calendars = createAsync(() => calendarsQuery());
  const [kind, setKind] = createSignal("confirmed");
  const [whole, setWhole] = createSignal(false);
  const own = () => (calendars() ?? []).filter((c) => !c.googleConnection);
  const add = async (e: SubmitEvent & { currentTarget: HTMLFormElement }) => {
    e.preventDefault();
    const f = new FormData(e.currentTarget);
    const text = (k: string) => String(f.get(k) ?? "").trim();
    const time = (day: string, at: string) => (whole() ? text(day) : `${text(day)} ${text(at)}`);
    const calendar = own().find((c) => c.id === text("calendar"));
    const fields = [
      { name: "Status", value: kind() },
      { name: "Start", value: time("from", "fromTime") },
      { name: "End", value: time("to", "toTime") },
      { name: "Location", value: text("location") },
      { name: "Meeting", value: text("meeting") },
      { name: "Calendar", value: calendar ? linkTo({ kind: "calendars", id: calendar.id, title: calendar.name }) : "" },
    ].filter((x) => x.value);
    const made = await act(
      () =>
        api.records.saveRecord({
          kind: "events",
          title: text("title"),
          notes: text("notes"),
          workgroups: newWorkgroups(workgroup()),
          fields,
        }),
      kind() === "hold" ? "仮押さえした" : "足した",
    );
    if (made) props.onOpenChange(false);
  };
  return (
    <Dialog
      open={props.open}
      onOpenChange={props.onOpenChange}
      title="予定を足す"
      description="手帳に置く予定。Google のカレンダーに入れるときは、会話で頼む。"
    >
      <Form class="dialog-form" onSubmit={add}>
        <Segmented
          label="予定の種類"
          items={[
            ["confirmed", "予定"],
            ["hold", "仮押さえ"],
          ]}
          value={kind()}
          onChoose={setKind}
        />
        <Field label="何の予定か">
          <TextField name="title" required autofocus />
        </Field>
        <Checkbox checked={whole()} onChange={(e) => setWhole(e.currentTarget.checked)}>
          終日
        </Checkbox>
        <Field label="始まり" group>
          <div class="row">
            <TextField type="date" name="from" value={props.day} required aria-label="始まりの日" />
            <Show when={!whole()}>
              <TextField type="time" name="fromTime" value="10:00" required aria-label="始まりの時刻" />
            </Show>
          </div>
        </Field>
        <Field label="終わり" group>
          <div class="row">
            <TextField type="date" name="to" value={props.day} required aria-label="終わりの日" />
            <Show when={!whole()}>
              <TextField type="time" name="toTime" value="11:00" required aria-label="終わりの時刻" />
            </Show>
          </div>
        </Field>
        <Field label="場所">
          <TextField name="location" />
        </Field>
        <Field label="会議の URL">
          <TextField name="meeting" type="url" />
        </Field>
        <Show when={own().length > 0}>
          <Field label="カレンダー">
            <Select name="calendar">
              <option value="">なし</option>
              <For each={own()}>{(c) => <option value={c.id}>{c.name}</option>}</For>
            </Select>
          </Field>
        </Show>
        <Field label="備考">
          <TextArea name="notes" placeholder="Markdown で書ける" />
        </Field>
        <DialogActions submit="足す" />
      </Form>
    </Dialog>
  );
}
