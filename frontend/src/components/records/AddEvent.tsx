import { createAsync } from "@solidjs/router";
import { createSignal, For, Show } from "solid-js";
import Modal from "~/components/Modal";
import Segs from "~/components/Segs";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { calendarsQuery } from "~/lib/calendar";
import { linkTo, newWorkgroups } from "~/lib/records";
import { workgroup } from "~/lib/workgroup";

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
      () => api.records.saveRecord({ kind: "events", title: text("title"), notes: text("notes"), workgroups: newWorkgroups(workgroup()), fields }),
      kind() === "hold" ? "仮押さえした" : "足した",
    );
    if (made) props.onOpenChange(false);
  };
  return (
    <Modal open={props.open} onOpenChange={props.onOpenChange} title="予定を足す" description="手帳に置く予定。Google のカレンダーに入れるときは、会話で頼む。">
      <form class="dialog-form" onSubmit={add}>
        <Segs
          label="予定の種類"
          items={[
            ["confirmed", "予定"],
            ["hold", "仮押さえ"],
          ]}
          value={kind()}
          onChoose={setKind}
        />
        <label class="dialog-field">
          <span>何の予定か</span>
          <input class="input" name="title" required autofocus />
        </label>
        <label class="records-over">
          <input type="checkbox" checked={whole()} onChange={(e) => setWhole(e.currentTarget.checked)} />
          終日
        </label>
        <div class="dialog-field">
          <span>始まり</span>
          <div class="row">
            <input class="input" type="date" name="from" value={props.day} required aria-label="始まりの日" />
            <Show when={!whole()}>
              <input class="input" type="time" name="fromTime" value="10:00" required aria-label="始まりの時刻" />
            </Show>
          </div>
        </div>
        <div class="dialog-field">
          <span>終わり</span>
          <div class="row">
            <input class="input" type="date" name="to" value={props.day} required aria-label="終わりの日" />
            <Show when={!whole()}>
              <input class="input" type="time" name="toTime" value="11:00" required aria-label="終わりの時刻" />
            </Show>
          </div>
        </div>
        <label class="dialog-field">
          <span>場所</span>
          <input class="input" name="location" />
        </label>
        <label class="dialog-field">
          <span>会議の URL</span>
          <input class="input" name="meeting" type="url" />
        </label>
        <Show when={own().length > 0}>
          <label class="dialog-field">
            <span>カレンダー</span>
            <select class="input" name="calendar">
              <option value="">なし</option>
              <For each={own()}>{(c) => <option value={c.id}>{c.name}</option>}</For>
            </select>
          </label>
        </Show>
        <label class="dialog-field">
          <span>備考</span>
          <textarea class="input" name="notes" placeholder="Markdown で書ける" />
        </label>
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
