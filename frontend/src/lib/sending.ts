import { type Accessor, createSignal } from "solid-js";
import { message, say } from "~/lib/act";

/**
 * One thing sent at a time. What is pressed while something is on its way is
 * not done, so a second press, an Enter or a tap sends nothing twice.
 */
export type Sending = {
  busy: Accessor<boolean>;
  /**
   * Do `start`. When it gives back a promise, nothing else is done until it
   * settles; with `hold`, nor after it did what it was for, as what it was
   * pressed on is gone or about to be. It failed when it throws or gives
   * `undefined` or `false`, as `act` does: then it can be done again, and a
   * throw is said.
   */
  run<T>(start: () => T, hold?: boolean): T | undefined;
};

function thenable(v: unknown): v is PromiseLike<unknown> {
  return typeof v === "object" && v !== null && typeof Reflect.get(v, "then") === "function";
}

export function createSending(): Sending {
  const [busy, setBusy] = createSignal(false);
  let sending = false;
  const release = () => {
    sending = false;
    setBusy(false);
  };
  return {
    busy,
    run(start, hold = false) {
      if (sending) return undefined;
      const result = start();
      if (!thenable(result)) return result;
      sending = true;
      setBusy(true);
      result.then(
        (done) => {
          if (!hold || done === undefined || done === false) release();
        },
        (e: unknown) => {
          release();
          say(message(e), true);
        },
      );
      return result;
    },
  };
}
