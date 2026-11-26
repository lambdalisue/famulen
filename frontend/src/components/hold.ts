import { createEffect, onCleanup } from "solid-js";

/** How far a finger goes before it is told whether it is held: less than the browser goes before it scrolls. */
const SLOP = 6;
/** How long a finger rests before it moves for the browser to take it as choosing text or moving the caret. */
const PRESS = 400;

/** Whether text is chosen where a finger is: its handles are moved there. A choice left elsewhere does not count. */
export function choosing(at: EventTarget | null) {
  const chosen = document.getSelection();
  if (!chosen || chosen.isCollapsed || chosen.rangeCount === 0 || !(at instanceof Node)) return false;
  return chosen.getRangeAt(0).intersectsNode(at);
}

/** What is laid over everything, outside what holds the page, and scrolls on its own. */
const OVER_ALL = "dialog[open], [role=dialog], [role=alertdialog]";

/** The nearest that scrolls up and down from where a touch is, and could go on scrolling as the finger goes. */
function scrolls(from: EventTarget | null, until: HTMLElement, down: boolean) {
  for (let at = from instanceof Element ? from : null; at && until.contains(at); at = at.parentElement) {
    if (at.scrollHeight <= at.clientHeight + 1 || !/auto|scroll/.test(getComputedStyle(at).overflowY)) continue;
    if (down ? at.scrollTop + at.clientHeight < at.scrollHeight - 1 : at.scrollTop > 0) return true;
  }
  return false;
}

/**
 * Keep a finger on what is laid over the page from scrolling anything else:
 * past the end of what it scrolls, over what does not scroll, and over the
 * rest of the screen. iOS rubber-bands the page behind otherwise, even where
 * `overscroll-behavior` asks it not to, and pans it while the keyboard is up.
 */
export function holdPageBehind(of: { over: () => HTMLElement | undefined; on: () => boolean }) {
  /** Where the finger came down, and once it has gone far enough, whether it is held for the rest of the way. */
  let touch: { x: number; y: number; at: number; held?: boolean } | undefined;
  const start = (e: TouchEvent) => {
    const t = e.touches[0];
    touch = e.touches.length === 1 && t ? { x: t.clientX, y: t.clientY, at: e.timeStamp } : undefined;
  };
  const decide = (e: TouchEvent, over: HTMLElement, dx: number, dy: number) => {
    // Text being chosen, or the caret moved, is the browser's to follow.
    if (e.timeStamp - (touch?.at ?? 0) > PRESS || choosing(e.target)) return false;
    // Sideways is what scrolls sideways, or a swipe back.
    if (Math.abs(dx) > Math.abs(dy)) return false;
    // A zoomed screen is panned, and what is open over everything, as an image, is its own.
    if ((window.visualViewport?.scale ?? 1) > 1 || (e.target instanceof Element && e.target.closest(OVER_ALL) && !over.contains(e.target))) return false;
    // The finger going up scrolls what is under it down.
    return !scrolls(e.target, over, dy < 0);
  };
  const move = (e: TouchEvent) => {
    const t = e.touches[0];
    const over = of.over();
    // Two fingers zoom, which is not held.
    if (e.touches.length !== 1) touch = undefined;
    if (!touch || !t || !over) return;
    if (touch.held === undefined) {
      const [dx, dy] = [t.clientX - touch.x, t.clientY - touch.y];
      if (Math.abs(dx) < SLOP && Math.abs(dy) < SLOP) return;
      touch.held = decide(e, over, dx, dy);
    }
    if (touch.held && e.cancelable) e.preventDefault();
  };
  createEffect(() => {
    if (!of.on()) return;
    document.addEventListener("touchstart", start, { passive: true });
    // The finger left of two that zoomed is held as one put down anew.
    document.addEventListener("touchend", start, { passive: true });
    // Not passive, so that the move can be kept from scrolling.
    document.addEventListener("touchmove", move, { passive: false });
    onCleanup(() => {
      document.removeEventListener("touchstart", start);
      document.removeEventListener("touchend", start);
      document.removeEventListener("touchmove", move);
      touch = undefined;
    });
  });
}
