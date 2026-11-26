import { Code, ConnectError } from "@connectrpc/connect";
import { batch, createSignal, onCleanup, onMount } from "solid-js";
import type { RunEntry, RunEvent, RunOutput } from "~/gen/famulen/v1/workers_pb";
import { api } from "./api";
import { addPart, type Parts, settle } from "./runview";

/** How long a dropped stream waits before it is opened again. */
const AGAIN_MS = 2000;
/** What no opening again would change. */
const GIVE_UP = [Code.NotFound, Code.PermissionDenied, Code.InvalidArgument, Code.Unauthenticated];
/** How often what came is taken in while the page is hidden. */
const HIDDEN_MS = 1000;

/**
 * Follow a run: how it stands, all it did, and what it is writing as it is
 * written. What was read first is shown until the stream brings it all anew.
 */
export function watchRun(id: () => string, first: () => RunOutput | undefined) {
  const [state, setState] = createSignal<RunOutput | undefined>();
  const [entries, setEntries] = createSignal<RunEntry[]>([]);
  const [parts, setParts] = createSignal<Parts>(new Map());
  const [live, setLive] = createSignal(false);
  const [seeded, setSeeded] = createSignal(false);
  let bySeq = new Map<bigint, RunEntry>();

  // What came since the screen was last drawn, taken in at once: a run read
  // anew brings thousands of entries, and what is written comes a word at a time.
  let waiting: (RunEvent | "dropped")[] = [];
  let pending: (() => void) | undefined;
  const flush = () => {
    pending = undefined;
    const came = waiting;
    waiting = [];
    batch(() => {
      let added = false;
      let now = parts();
      for (const e of came) {
        if (e === "dropped") {
          now = new Map();
          continue;
        }
        switch (e.event.case) {
          case "state":
            setState(e.event.value);
            break;
          case "entry":
            bySeq.set(e.event.value.seq, e.event.value);
            now = settle(now, e.event.value);
            added = true;
            break;
          case "part":
            now = addPart(now, e.event.value);
            break;
          case "reset":
            now = new Map();
            break;
        }
      }
      setParts(now);
      if (added) {
        setEntries([...bySeq.values()].sort((a, b) => (a.seq < b.seq ? -1 : 1)));
        setSeeded(true);
      }
    });
  };
  const take = (e: RunEvent | "dropped") => {
    waiting.push(e);
    if (pending) return;
    // A hidden page draws nothing, and would keep all that comes until it is shown.
    if (document.hidden) {
      const timer = setTimeout(flush, HIDDEN_MS);
      pending = () => clearTimeout(timer);
    } else {
      const frame = requestAnimationFrame(flush);
      pending = () => cancelAnimationFrame(frame);
    }
  };

  const seed = (o: RunOutput | undefined) => {
    if (!o || bySeq.size > 0) return;
    bySeq = new Map(o.entries.map((e) => [e.seq, e]));
    setEntries(o.entries);
    setState(o);
    setSeeded(true);
  };

  onMount(() => {
    let stopped = false;
    let abort: AbortController | undefined;
    const run = async () => {
      seed(first());
      while (!stopped) {
        abort = new AbortController();
        // All that was kept is read anew each time it is opened: an entry may
        // have grown under its number since it was read.
        const opened = api.workers.watchRun({ id: BigInt(id()), after: 0n }, { signal: abort.signal });
        try {
          for await (const e of opened) {
            setLive(true);
            take(e);
          }
        } catch (error) {
          // A run that is not there, or not to be watched, is not asked again.
          if (error instanceof ConnectError && GIVE_UP.includes(error.code)) stopped = true;
          // Otherwise the stream ended or the backend restarted; it is opened again below.
        }
        setLive(false);
        // What was being written is told whole once it settles; a part cut off is dropped.
        take("dropped");
        if (!stopped) await new Promise((r) => setTimeout(r, AGAIN_MS));
      }
    };
    void run();
    onCleanup(() => {
      stopped = true;
      abort?.abort();
      pending?.();
    });
  });

  return {
    state: () => state() ?? first(),
    // Before the stream is followed, what was read first: the same on the server and in the browser.
    entries: () => (seeded() ? entries() : (first()?.entries ?? [])),
    parts,
    live,
  };
}
