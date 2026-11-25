import { onCleanup } from "solid-js";

/** The widths of `--phone` in app.css, for what a script decides. */
const NARROW = "(max-width: 700px)";

export const isNarrow = () => window.matchMedia(NARROW).matches;

let held = false;
let follow = () => {};

/** Whether the layout stands still though the keyboard goes. */
export const layoutHeld = () => held;

/** Keep the layout as it is with or without the keyboard, until it is freed. */
export function holdLayout() {
  held = true;
}

/** Let the layout follow the keyboard again; `returning` when it is on its way back. */
export function freeLayout(returning: boolean) {
  held = false;
  // The screen as it is before the keyboard is back is not to be followed, but
  // a keyboard that stays away, as with a hardware one, is.
  if (returning) setTimeout(() => follow(), 1000);
  else follow();
}

/**
 * Keep where the part of the screen over an on-screen keyboard is in CSS, as
 * iOS lays the page out as though the keyboard were not there.
 */
export function followKeyboard() {
  const seen = window.visualViewport;
  if (!seen) return;
  const put = () => {
    if (held) return;
    const root = document.documentElement.style;
    root.setProperty("--seen-top", `${seen.offsetTop}px`);
    root.setProperty("--seen-height", `${seen.height}px`);
  };
  follow = put;
  put();
  seen.addEventListener("resize", put);
  seen.addEventListener("scroll", put);
  onCleanup(() => {
    seen.removeEventListener("resize", put);
    seen.removeEventListener("scroll", put);
  });
}
