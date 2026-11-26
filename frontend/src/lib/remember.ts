import { query, revalidate, useIsRouting } from "@solidjs/router";
import { getOwner, untrack } from "solid-js";
import { isServer } from "solid-js/web";

/** How many reads of one query are kept to show again; the oldest goes first. */
const KEPT = 32;

/** What each remembered query read last, by its key. */
const kept: Map<string, unknown>[] = [];

/** Whether the page is changing: only then may what was read before stand in. */
function changingPage(): boolean {
  if (!getOwner()) return false;
  try {
    return untrack(useIsRouting());
  } catch {
    // Read outside the router, such as while a component is tried alone.
    return false;
  }
}

/**
 * Read again what is on screen after something changed, and forget what was
 * read before for any page, so that coming back to one does not show what
 * was there before the change. With no keys, everything is.
 */
export function reread(keys?: string | string[]): Promise<void> {
  const of = keys === undefined ? undefined : Array.isArray(keys) ? keys : [keys];
  for (const last of kept) {
    for (const key of [...last.keys()]) if (!of || of.some((k) => key.startsWith(k))) last.delete(key);
  }
  return revalidate(keys);
}

/**
 * A query that, when the owner comes back to a page, shows at once what it
 * read last and reads again behind it, so the page does not wait for the
 * backend before it appears. Anything else that reads it again, such as a
 * change or a record changed elsewhere, still waits for what is there now,
 * so a page is never left showing what was before.
 */
export function remembered<A extends unknown[], T>(fn: (...args: A) => Promise<T>, name: string) {
  const last = new Map<string, T>();
  kept.push(last);
  // Which read of a key is the newest; an older one that ends later is not shown.
  const reads = new Map<string, number>();
  const read = query(async (...args: A): Promise<T> => {
    if (isServer) return fn(...args);
    const key = read.keyFor(...args);
    const had = last.get(key);
    const turn = (reads.get(key) ?? 0) + 1;
    reads.set(key, turn);
    const now = fn(...args).then((value) => {
      if (reads.get(key) === turn) {
        last.delete(key);
        last.set(key, value);
        if (last.size > KEPT) last.delete(last.keys().next().value ?? key);
      }
      return value;
    });
    if (had === undefined || !changingPage()) return now;
    now.then(
      (value) => {
        if (reads.get(key) !== turn) return;
        query.set<unknown>(key, value);
        void revalidate(key, false);
      },
      () => {
        // Read once more the way a page does, so that it shows why it could not.
        last.delete(key);
        void revalidate(key);
      },
    );
    return had;
  }, name);
  return read;
}
