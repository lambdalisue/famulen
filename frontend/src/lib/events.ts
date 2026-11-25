import { revalidate } from "@solidjs/router";
import { createEffect, createSignal, on, onCleanup } from "solid-js";
import type { ConsoleMessage, Working } from "~/gen/famulen/v1/console_pb";
import { api } from "./api";

const [incoming, setIncoming] = createSignal<{ of: string; m: ConsoleMessage } | undefined>(undefined, { equals: false });

/** The newest message pushed while the console is open, and the workgroup it was pushed for. */
export const pushed = incoming;

const [doing, setDoing] = createSignal("");

/** What the secretary is doing for the owner, in words; empty when there is nothing to show. */
export const working = doing;

let stale: ReturnType<typeof setTimeout> | undefined;

function show(w: Working) {
  clearTimeout(stale);
  setDoing(w.text);
  if (w.text) stale = setTimeout(() => setDoing(""), w.seconds * 1000);
}

/**
 * Follow what changes on the backend: new messages arrive as they are said,
 * and every list on screen reloads when a record or the inbox changes.
 * A dropped stream is opened again, and so is one for another workgroup.
 */
export function followEvents(workgroup: () => string) {
  let stopped = false;
  let abort: AbortController | undefined;
  const run = async () => {
    while (!stopped) {
      abort = new AbortController();
      const of = workgroup();
      try {
        for await (const e of api.console.subscribeEvents({ workgroup: of }, { signal: abort.signal })) {
          switch (e.event.case) {
            case "message":
              setIncoming({ of, m: e.event.value });
              break;
            case "working":
              show(e.event.value);
              break;
            case "inbox":
              await revalidate("inbox");
              break;
            case "records":
              await revalidate();
              break;
          }
        }
      } catch {
        // The stream ended or the backend restarted; reconnect below.
      }
      if (!stopped && !abort.signal.aborted) await new Promise((r) => setTimeout(r, 2000));
    }
  };
  void run();
  createEffect(on(workgroup, () => abort?.abort(), { defer: true }));
  onCleanup(() => {
    stopped = true;
    abort?.abort();
  });
}
