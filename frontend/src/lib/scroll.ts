/** What is left above a message that is gone to. */
const ABOVE = 8;

/** How far a list is to be scrolled so that `to` is at its top. */
export function offsetIn(list: HTMLElement, to: HTMLElement): number {
  return to.getBoundingClientRect().top - list.getBoundingClientRect().top - ABOVE;
}

/** Show for a moment which message it is. */
export function pointOut(message: HTMLElement) {
  message.animate([{ background: "var(--accent-soft)" }, { background: "transparent" }], { duration: 1600, easing: "ease-in" });
}
