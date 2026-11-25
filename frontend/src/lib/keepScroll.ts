import { useBeforeLeave } from "@solidjs/router";
import { onMount } from "solid-js";

const left = new Map<string, number>();

function scroller(el: Element | null): Element | undefined {
  for (let at = el; at; at = at.parentElement) {
    if (at.scrollHeight > at.clientHeight && /auto|scroll/.test(getComputedStyle(at).overflowY)) return at;
  }
  return undefined;
}

/** Come back to where a list was read when what was opened from it, under `to`, is left. */
export function keepScroll(name: string, inside: () => Element, to: string) {
  useBeforeLeave((e) => {
    left.delete(name);
    if (typeof e.to === "string" && e.to.startsWith(to)) left.set(name, scroller(inside())?.scrollTop ?? 0);
  });
  onMount(() => {
    const top = left.get(name);
    left.delete(name);
    if (!top) return;
    // The router puts a new page at its top first.
    requestAnimationFrame(() => scroller(inside())?.scrollTo({ top }));
  });
}
