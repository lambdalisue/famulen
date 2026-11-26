import { For, Show } from "solid-js";
import { Button, Form, TextField } from "~/components/ui";
import type { Rec } from "~/lib/records";
import LinkedText from "../links/LinkedText";

/** What happened to a record, newest first, and a line to add to it. */
export default function Log(props: { record: Rec; aide: string; onAdd: (line: string) => Promise<boolean> }) {
  const who = (w: string) => ({ owner: "あなた", secretary: props.aide, famulen: "Famulen" })[w] ?? w;
  const lines = () => [...props.record.log].reverse();
  return (
    <section class="rec-section">
      <h3>経過</h3>
      <Form
        class="rec-new"
        onSubmit={(e) => {
          e.preventDefault();
          const box = e.currentTarget.querySelector("input");
          if (!box?.value.trim()) return;
          return props.onAdd(box.value.trim()).then((ok) => {
            if (ok) box.value = "";
          });
        }}
      >
        <TextField class="grow" placeholder="あったことを書く" aria-label="経過を足す" />
        <Button type="submit" small>
          足す
        </Button>
      </Form>
      <ol class="rec-log">
        <For each={lines()}>
          {(l) => (
            <li>
              <span class="sub mono">{l.at}</span>
              <span class="rec-log-text">
                <LinkedText text={l.text} />
              </span>
              <Show when={l.who}>
                <span class="sub">{who(l.who)}</span>
              </Show>
            </li>
          )}
        </For>
      </ol>
    </section>
  );
}
