import { createAsync, query, revalidate } from "@solidjs/router";
import { createEffect, on } from "solid-js";
import { type ConsoleMessage, ConsoleMessageSchema } from "~/gen/famulen/v1/console_pb";
import { api } from "./api";
import { merged } from "./conversation";
import { pushed } from "./events";
import { createSame } from "./kept";

type Read = { workgroup: string; root: bigint; messages: ConsoleMessage[]; error?: unknown };

// The id is given as text, as what a query is keyed by cannot hold a bigint.
const threadQuery = query(async (workgroup: string, id: string): Promise<Read> => {
  const root = BigInt(id);
  const { messages } = await api.console.listMessages({ workgroup, thread: root });
  return { workgroup, root, messages };
}, "thread");

/**
 * The thread that begins with the message `root`, oldest first: as the
 * conversation holds it, and as it was read again since.
 */
export function createThread(workgroup: () => string, root: () => bigint | undefined, said: () => ConsoleMessage[]) {
  const read = createAsync<Read | undefined>(async (before) => {
    const of = workgroup();
    const id = root();
    if (id === undefined) return undefined;
    return threadQuery(of, id.toString()).catch((error: unknown) => {
      const had = before?.root === id && before.workgroup === of ? before.messages : [];
      return { workgroup: of, root: id, messages: had, error };
    });
  });
  createEffect(
    on(
      pushed,
      (came) => {
        const id = root();
        if (id !== undefined && came && (came.m.id === id || came.m.thread === id)) void revalidate("thread");
      },
      { defer: true },
    ),
  );
  const now = () => {
    const r = read.latest;
    return r && r.root === root() && r.workgroup === workgroup() ? r : undefined;
  };
  const messages = createSame(
    ConsoleMessageSchema,
    () => {
      const id = root();
      if (id === undefined) return [];
      return merged(
        said().filter((m) => m.id === id || m.thread === id),
        now()?.messages ?? [],
      );
    },
    (m) => m.id,
  );
  return { messages, failed: () => now()?.error, retry: () => revalidate("thread") };
}
