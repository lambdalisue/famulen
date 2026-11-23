import { createAsync, revalidate } from "@solidjs/router";
import { Show, createEffect, createSignal, onCleanup } from "solid-js";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import * as icons from "~/lib/icons";
import { dreamsQuery, layersQuery } from "~/lib/memory";
import { Button, Tag } from "~/components/ui";

/** How long to wait for a dream asked for to show up before saying it did not start. */
const START_WAIT_MS = 20000;

/**
 * 今すぐ夢を見る, with whether a dream runs now or why none can. `linkWhy`
 * makes the reason a link to the dream's settings, for pages other than those.
 */
export default function DreamNow(props: { linkWhy?: boolean }) {
  const dreams = createAsync(() => dreamsQuery());
  // Pressing the button starts the dream in the background, so the backend
  // may not say it is dreaming yet on the next read; until it does (or a new
  // night appears) the button keeps saying it has started.
  const [asked, setAsked] = createSignal<{ at: number; last: bigint | undefined; seen: boolean }>();
  const lastId = () => dreams()?.dreams.at(-1)?.id;
  const running = () => dreams()?.dreaming === true || asked() !== undefined;

  createEffect(() => {
    const a = asked();
    const d = dreams();
    if (!a || !d) return;
    if (d.dreaming) {
      if (!a.seen) setAsked({ ...a, seen: true });
      return;
    }
    if (a.seen || lastId() !== a.last || Date.now() - a.at > START_WAIT_MS) setAsked(undefined);
  });

  createEffect(() => {
    if (!running()) return;
    const timer = setInterval(() => {
      void revalidate([dreamsQuery.key, layersQuery.key]);
    }, 3000);
    onCleanup(() => clearInterval(timer));
  });

  const dreamNow = async () => {
    const last = lastId();
    const done = await act(() => api.memory.dreamNow({}), "夢を見はじめた");
    if (done) setAsked({ at: Date.now(), last, seen: false });
  };

  return (
    <span class="row dream-now">
      <Show when={running()}>
        <Tag role="status">いま見ている…</Tag>
      </Show>
      <Show when={!running() && dreams()?.blocked}>
        {(why) => (
          <Show when={props.linkWhy} fallback={<Tag tone="warn">{why()}</Tag>}>
            <a class="tag warn" href="/settings/roles/dream">
              {why()}
            </a>
          </Show>
        )}
      </Show>
      <Button icon={icons.refresh} disabled={running() || !!dreams()?.blocked} onClick={dreamNow}>
        今すぐ夢を見る
      </Button>
    </span>
  );
}
