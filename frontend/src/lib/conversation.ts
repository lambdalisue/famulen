import { createAsync, query, revalidate } from "@solidjs/router";
import { batch, createEffect, createMemo, createSignal, on, untrack } from "solid-js";
import { type ConsoleMessage, ConsoleMessageSchema } from "~/gen/famulen/v1/console_pb";
import { api } from "./api";
import { pushed } from "./events";
import { createSame } from "./kept";

/** How many messages are read at a time. */
const PAGE = 50;
/** How many are read at a time to catch up with what was missed. */
const CATCH_UP = 200;

type Read = { workgroup: string; messages: ConsoleMessage[]; more: boolean; error?: unknown };

/** A message that came another way than with the newest, and the workgroup it came in. */
type Came = { of: string; m: ConsoleMessage };
/** The newest messages, with all that had come another way when they were asked for. */
type Newest = Read & { had: Came[] };

/** The newest message read of the workgroup on screen. Only ever set in the browser, as the server has many. */
let reached: { workgroup: string; id: bigint } | undefined;

/** The newest messages, back to those read before if there are any, so that none between is missing. */
const newestQuery = query(async (workgroup: string): Promise<Read> => {
  const known = reached?.workgroup === workgroup ? reached.id : undefined;
  let { messages, more } = await api.console.listMessages({ workgroup, limit: PAGE });
  while (known !== undefined && more && messages[0] && messages[0].id > known) {
    const older = await api.console.listMessages({ workgroup, before: messages[0].id, limit: CATCH_UP });
    messages = [...older.messages, ...messages];
    more = older.more;
  }
  return { workgroup, messages, more };
}, "messages");

/** Both by id, oldest first. One in `later` is how it stands now. */
export function merged(before: ConsoleMessage[], later: ConsoleMessage[]): ConsoleMessage[] {
  const by = new Map<bigint, ConsoleMessage>();
  for (const m of [...before, ...later]) by.set(m.id, m);
  return [...by.values()].sort((a, b) => Number(a.id - b.id));
}

/**
 * The conversation in a workgroup, newest first: the newest messages are read
 * at once, older ones a page at a time when asked for, and new ones come in as
 * they are said. Once the newest are read again while the owner `follows`
 * them, older ones are let go of.
 */
export function createConversation(workgroup: () => string, follows: () => boolean) {
  const [others, setOthers] = createSignal<Came[]>([]);
  const read = createAsync<Newest>((before) => {
    const of = workgroup();
    const had = untrack(others);
    return newestQuery(of).then(
      (now) => ({ ...now, had }),
      // What was read before stands when a later reading fails.
      (e: unknown) => {
        if (!before) throw e;
        return before.workgroup === of ? before : { workgroup: of, messages: [], more: true, had, error: e };
      },
    );
  });
  // Nothing is taken off the screen while it is read again.
  const newest = () => read.latest;
  // `start` counts the times it was begun from the newest. What was read back to before that is let go of.
  const kept = createMemo<(Read & { start: number }) | undefined>((was) => {
    const now = newest();
    if (!now) return was;
    if (!was || was.workgroup !== now.workgroup || was.messages.length === 0 || follows()) return { ...now, start: (was?.start ?? 0) + 1 };
    return { ...was, messages: merged(was.messages, now.messages) };
  });

  // Memos, as an effect on them is to run only when they are another one.
  const of = createMemo(() => kept()?.workgroup);
  const start = createMemo(() => kept()?.start ?? 0);

  const [readTo, setReadTo] = createSignal<{ start: number; to: bigint; more: boolean }>();
  const back = () => {
    const at = readTo();
    return at?.start === start() ? at : undefined;
  };
  const more = () => back()?.more ?? kept()?.more ?? false;
  /** The oldest message read. Nothing between it and the newest is missing. */
  const oldest = () => back()?.to ?? kept()?.messages[0]?.id;
  /** An older one pushed, such as once its files came in, is left until it is read in its place. */
  const joins = (id: bigint) => !more() || id >= (oldest() ?? id + 1n);

  const messages = createSame(
    ConsoleMessageSchema,
    () =>
      merged(
        kept()?.messages ?? [],
        others().flatMap((o) => (o.of === of() && joins(o.m.id) ? [o.m] : [])),
      ),
    (m) => m.id,
  );
  const add = (of: string, m: ConsoleMessage) => setOthers((o) => [...o.filter((x) => x.of !== of || x.m.id !== m.id), { of, m }]);

  const [reading, setReading] = createSignal(false);
  const [missed, setMissed] = createSignal<unknown>();
  const failed = () => missed() ?? kept()?.error;

  createEffect(
    on(
      start,
      () => {
        setReading(false);
        setMissed(undefined);
        const first = kept()?.messages[0]?.id ?? 0n;
        setOthers((o) => o.filter((x) => x.of !== of() || x.m.id >= first));
      },
      { defer: true },
    ),
  );
  createEffect(() => {
    const now = newest();
    if (!now) return;
    const last = now.messages.at(-1);
    reached = last && { workgroup: now.workgroup, id: last.id };
    const ids = new Set(now.messages.map((m) => m.id));
    const had = new Set(now.had);
    setOthers((o) => o.filter((x) => x.of === now.workgroup && !(had.has(x) && ids.has(x.m.id))));
  });
  createEffect(
    on(
      pushed,
      (came) => {
        if (came) add(came.of, came.m);
      },
      { defer: true },
    ),
  );

  let underway: Promise<void> | undefined;
  /** Read the page before the oldest message; one that failed is only tried again by `retry`. */
  const older = (limit = PAGE): Promise<void> => {
    if (reading() && underway) return underway;
    underway = readOlder(limit);
    return underway;
  };
  const readOlder = async (limit: number) => {
    const here = of();
    const before = oldest();
    if (here === undefined || before === undefined || !more() || reading() || failed()) return;
    const mine = start();
    setReading(true);
    try {
      const page = await api.console.listMessages({ workgroup: here, before, limit });
      if (mine !== start()) return;
      batch(() => {
        setOthers((o) => {
          const came = new Set(o.flatMap((x) => (x.of === here ? [x.m.id] : [])));
          return [...page.messages.filter((m) => !came.has(m.id)).map((m) => ({ of: here, m })), ...o];
        });
        setReadTo({ start: mine, to: page.messages[0]?.id ?? before, more: page.more });
      });
    } catch (e) {
      if (mine === start()) setMissed(e);
    } finally {
      if (mine === start()) setReading(false);
    }
  };
  /** Read back until the message `id` is there, and say whether it is. */
  const reach = async (id: bigint): Promise<boolean> => {
    const here = of();
    const there = () => messages().some((m) => m.id === id);
    while (!there()) {
      const from = oldest();
      if (from === undefined || from < id || !more() || failed()) return false;
      await older(CATCH_UP);
      if (of() !== here) return false;
    }
    return true;
  };
  const retry = async () => {
    if (kept()?.error) await revalidate("messages");
    setMissed(undefined);
    await older();
  };

  return {
    /** The workgroup the messages are of, once they are there. */
    workgroup: of,
    /** Oldest first. */
    messages,
    /** Show what was just sent in `of`, unless it was pushed already. */
    sent: (of: string, m: ConsoleMessage) => {
      if (!others().some((x) => x.of === of && x.m.id === m.id)) add(of, m);
    },
    more,
    reading,
    failed,
    older: () => older(),
    reach,
    retry,
  };
}
