import { createSignal } from "solid-js";
import { isServer } from "solid-js/web";

const [view, setViewSignal] = createSignal("");
const [selection, setSelection] = createSignal("");

/** What the owner is looking at, sent along with what they say. */
export { view };

/**
 * Name what the page shows. On the server this does nothing: one signal
 * serves every request there, so a page would show another's name. In the
 * browser the write waits for the render to finish, since a page names
 * itself while hydrating and a write then would not reach the chat.
 */
export function setView(text: string) {
  if (!isServer) queueMicrotask(() => setViewSignal(text));
}

/** The item the owner picked on screen, sent along until removed. */
export { selection, setSelection };
