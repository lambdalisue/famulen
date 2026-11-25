import { createAsync, query, revalidate } from "@solidjs/router";
import { createEffect, createMemo, createSignal, on, onCleanup } from "solid-js";
import { type Conversation, ConversationSchema, type ConsoleMessage, ConsoleMessageSchema } from "~/gen/famulen/v1/console_pb";
import { api } from "./api";
import { merged } from "./conversation";
import { pushed, working } from "./events";
import { createSame } from "./kept";
import { keep, kept } from "./storage";

/** What the conversation column shows: every conversation, one, the beginning of another, or all that was said in one line. */
export type Place = { kind: "list" } | { kind: "new" } | { kind: "all" } | { kind: "talk"; id: bigint };

const KEY = "famulen.talk";

const [place, setPlace] = createSignal<Place>({ kind: "list" });
/** The conversation that was open last, to go on with. */
const [lastOpen, setLastOpen] = createSignal<bigint>();

export { lastOpen, place };

/** Another workgroup is chosen: what was open last is not gone on with from there. */
export const forgetLast = () => setLastOpen();

export function go(to: Place) {
  if (to.kind === "talk") setLastOpen(to.id);
  setPlace(to);
  keep(KEY, to.kind === "talk" ? to.id.toString() : to.kind === "new" ? "list" : to.kind);
}

/** Open a conversation, or the list when it is not known which. */
export const openTalk = (id: bigint) => go(id ? { kind: "talk", id } : { kind: "list" });

/** Put back what was shown when the page was left. Only in the browser, once the page has been taken over. */
export function restorePlace() {
  const was = kept(KEY);
  if (was === "all") go({ kind: "all" });
  else if (was && /^\d+$/.test(was)) openTalk(BigInt(was));
}

const PAGE = 30;

type Read = { workgroup: string; also: bigint; talks: Conversation[]; more: boolean };

/** How long changes are gathered before the list is read again. */
const GATHER = 250;

const talksQuery = query(async (workgroup: string, limit: number, also: string): Promise<Read> => {
  const { conversations, more } = await api.console.listConversations({ workgroup, limit, also: BigInt(also) });
  return { workgroup, also: BigInt(also), talks: conversations, more };
}, "conversations");

/** The secretary's conversations, the one used last first, with `also` however long ago it was used. They are read again as something is said or done in one. */
export function createTalks(workgroup: () => string, also: () => bigint | undefined) {
  const [limit, setLimit] = createSignal(PAGE);
  const [failed, setFailed] = createSignal<unknown>();
  const read = createAsync<Read | undefined>((before) =>
    talksQuery(workgroup(), limit(), (also() ?? 0n).toString()).then(
      (now) => {
        setFailed(undefined);
        return now;
      },
      (e: unknown) => {
        setFailed(e);
        return before;
      },
    ),
  );
  let soon: ReturnType<typeof setTimeout> | undefined;
  const again = () => {
    clearTimeout(soon);
    soon = setTimeout(() => void revalidate("conversations"), GATHER);
  };
  onCleanup(() => clearTimeout(soon));
  createEffect(on(pushed, (came) => came && again(), { defer: true }));
  createEffect(on(working, again, { defer: true }));
  const now = () => (read.latest?.workgroup === workgroup() ? read.latest : undefined);
  const talks = createSame(ConversationSchema, () => now()?.talks ?? [], (c) => c.id);
  return {
    talks,
    /** Whether they were read. */
    there: () => now() !== undefined,
    /** Whether the one to be told besides was asked for, and the workgroup does not show it. */
    lacks: (id: bigint) => now()?.also === id && !talks().some((c) => c.id === id),
    more: () => now()?.more ?? false,
    further: () => setLimit((l) => l + PAGE),
    of: (id: bigint) => talks().find((c) => c.id === id),
    failed,
    retry: again,
  };
}

type Said = { workgroup: string; id: bigint; opening: number; readTo: bigint; messages: ConsoleMessage[]; error?: unknown };

// The id is given as text, as what a query is keyed by cannot hold a bigint.
const saidQuery = query(async (workgroup: string, id: string, opening: number): Promise<Said> => {
  const conversation = BigInt(id);
  const { messages, readTo } = await api.console.listMessages({ workgroup, conversation });
  return { workgroup, id: conversation, opening, readTo, messages };
}, "conversation");

/** What is said in one conversation, oldest first, with what comes in as it is said. */
export function createTalk(workgroup: () => string, id: () => bigint | undefined) {
  const [came, setCame] = createSignal<ConsoleMessage[]>([]);
  // Each opening reads anew, as what came meanwhile was not kept.
  let openings = 0;
  const opened = createMemo(() => {
    const which = id();
    return which === undefined ? undefined : { id: which, opening: ++openings };
  });
  const read = createAsync<Said | undefined>(async (before) => {
    const of = workgroup();
    const now = opened();
    if (!now) return undefined;
    return saidQuery(of, now.id.toString(), now.opening).catch((error: unknown) => {
      const had = before?.id === now.id && before.workgroup === of ? before : undefined;
      return { workgroup: of, ...now, readTo: had?.readTo ?? 0n, messages: had?.messages ?? [], error };
    });
  });
  createEffect(on([workgroup, id], () => setCame([]), { defer: true }));
  const add = (m: ConsoleMessage) => setCame((all) => [...all.filter((x) => x.id !== m.id), m]);
  createEffect(
    on(
      pushed,
      (p) => {
        const which = id();
        if (which === undefined || !p || p.of !== workgroup()) return;
        if (p.m.conversation === which) add(p.m);
        // One pushed before it was known which conversation it is of.
        else if (!p.m.conversation) void revalidate("conversation");
      },
      { defer: true },
    ),
  );
  const now = createMemo(() => {
    const r = read.latest;
    return r && r.opening === opened()?.opening && r.workgroup === workgroup() ? r : undefined;
  });
  const messages = createSame(ConsoleMessageSchema, () => merged(now()?.messages ?? [], came()), (m) => m.id);
  return {
    messages,
    /** Show what was just sent, unless it was pushed already. */
    sent: add,
    there: () => now() !== undefined,
    /** Which opening it is, and how far the owner had read as it began. */
    opening: () => opened()?.opening,
    readTo: () => now()?.readTo,
    failed: () => now()?.error,
    retry: () => revalidate("conversation"),
  };
}

/** Where a conversation that the owner did not begin came from, in words. */
export const SOURCES: { [source: string]: string } = {
  slack: "Slack",
  schedule: "定期の仕事",
  wake: "起床予約",
  arrival: "つなぎ先の知らせ",
  lookout: "見張り",
  approval: "確認の答え",
  worker: "委託の終わり",
};

export const titleOf = (c: Conversation | undefined) => c?.title || "（題のない話）";
