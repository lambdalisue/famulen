import { equals, type DescMessage, type MessageShape } from "@bufbuild/protobuf";
import { createAsync } from "@solidjs/router";
import { createMemo } from "solid-js";

/**
 * What a query says, read again as it changes. What was read before stands
 * when a later reading fails, so that a phone out of reach for a moment does
 * not take down what is around every page.
 */
export function createKept<T>(read: () => Promise<T>) {
  return createAsync<T>((before) =>
    read().catch((e: unknown) => {
      if (before === undefined) throw e;
      return before;
    }),
  );
}

/**
 * What is read in a dialog or a sheet that is open: undefined until it is read
 * first, then what it said last while it is read again. Waiting for it by the
 * Suspense around would take the form off the screen, and the focus with it.
 */
export function createLate<T>(read: () => Promise<T>) {
  const late = createAsync<T | undefined>(read, { initialValue: undefined });
  return () => late.latest;
}

/**
 * A list read again, with each item that says what it said before as the same
 * object, so that `For` leaves its elements, and what the owner does in them, alone.
 */
export function createSame<D extends DescMessage>(schema: D, read: () => MessageShape<D>[], id: (item: MessageShape<D>) => bigint) {
  return createMemo<MessageShape<D>[]>((before) => {
    const had = new Map(before.map((item) => [id(item), item]));
    return read().map((item) => {
      const old = had.get(id(item));
      return old && equals(schema, old, item) ? old : item;
    });
  }, []);
}
