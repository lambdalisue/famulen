import { createSignal } from "solid-js";
import { isServer } from "solid-js/web";

/** A part of a view's name: what the secretary is told, and what the owner reads when that differs. */
type Part = string | { told: string; shown: string };

const told = (p: Part) => (typeof p === "string" ? p : p.told);
const shown = (p: Part) => (typeof p === "string" ? p : p.shown);

const [parts, setParts] = createSignal<Part[]>([]);
const [selection, setSelection] = createSignal("");

/** What the owner is looking at, sent along with what they say. */
export const view = () => parts().map(told).join(" · ");

/** What the page shows, as the narrow layout heads it; empty when it is a whole section. */
export const pageTitle = () => (parts().length > 1 ? shown(parts()[parts().length - 1]) : "");

/**
 * Name what the page shows, its section first. On the server this does
 * nothing: one signal serves every request there, so a page would show
 * another's name. In the browser the write waits for the render to finish,
 * since a page names itself while hydrating and a write then would not reach
 * the chat.
 */
export function setView(...names: (Part | undefined)[]) {
  if (!isServer) queueMicrotask(() => setParts(names.filter((n): n is Part => !!n)));
}

/** The item the owner picked on screen, sent along until removed. */
export { selection, setSelection };
