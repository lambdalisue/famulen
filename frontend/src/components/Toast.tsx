import { Show } from "solid-js";
import { toast } from "~/lib/act";

export default function Toast() {
  return (
    <Show when={toast()}>
      {(t) => (
        <div id="toast" class={t().bad ? "show bad" : "show"} role="status">
          {t().text}
        </div>
      )}
    </Show>
  );
}
