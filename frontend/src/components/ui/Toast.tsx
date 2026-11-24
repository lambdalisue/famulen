import { Show } from "solid-js";
import { toast } from "~/lib/act";

/** The short note at the bottom of the screen after a change. */
export default function Toast() {
  return (
    <div class="toasts" aria-live="polite">
      <Show when={toast()}>
        {(t) => (
          <div id="toast" class={t().bad ? "bad" : ""} role={t().bad ? "alert" : undefined}>
            {t().text}
          </div>
        )}
      </Show>
    </div>
  );
}
