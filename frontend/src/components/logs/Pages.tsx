import { createAsync } from "@solidjs/router";
import { createSignal, Show } from "solid-js";
import Icon from "~/components/Icon";
import * as icons from "~/lib/icons";

/** Rows on one page of a log in the log sheet. */
export const PAGE = 30;

/** One page of a log as the API answers it, newest first. */
export type Page<T> = { rows: T[]; more: boolean };

/**
 * A log read a page at a time, newest first. `read` is given the id of the
 * oldest row shown so far (0 for the newest page) and is expected to call a
 * router query, so that a change made elsewhere reloads the page on screen.
 */
export function createPages<T extends { id: bigint }>(read: (before: number) => Promise<Page<T>>) {
  // Ids stay numbers here: a query's key is JSON, which cannot hold a bigint.
  const [cursors, setCursors] = createSignal<number[]>([0]);
  const page = createAsync(() => read(cursors().at(-1) ?? 0));
  return {
    rows: () => page()?.rows ?? [],
    loaded: () => page() !== undefined,
    more: () => page()?.more ?? false,
    number: () => cursors().length,
    older: () => {
      const last = page()?.rows.at(-1);
      if (last) setCursors((c) => [...c, Number(last.id)]);
    },
    newer: () => setCursors((c) => (c.length > 1 ? c.slice(0, -1) : c)),
    reset: () => setCursors([0]),
  };
}

/** The buttons that turn to the newer and the older page. */
export function PageNav(props: { pages: ReturnType<typeof createPages<{ id: bigint }>> }) {
  return (
    <Show when={props.pages.number() > 1 || props.pages.more()}>
      <nav class="log-pages" aria-label="頁送り">
        <button
          type="button"
          class="btn small"
          disabled={props.pages.number() === 1}
          onClick={() => props.pages.newer()}
        >
          <Icon svg={icons.chevronLeft} size={14} />
          新しいほうへ
        </button>
        <span class="sub">{props.pages.number()} 頁目</span>
        <button type="button" class="btn small" disabled={!props.pages.more()} onClick={() => props.pages.older()}>
          古いほうへ
          <Icon svg={icons.chevronRight} size={14} />
        </button>
      </nav>
    </Show>
  );
}
