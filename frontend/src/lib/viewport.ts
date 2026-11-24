import { onCleanup } from "solid-js";

/** The widths of `--phone` in app.css, for what a script decides. */
const NARROW = "(max-width: 700px)";

export const isNarrow = () => window.matchMedia(NARROW).matches;

/**
 * Keep where the part of the screen over an on-screen keyboard is in CSS, as
 * iOS lays the page out as though the keyboard were not there.
 */
export function followKeyboard() {
  const seen = window.visualViewport;
  if (!seen) return;
  const put = () => {
    const root = document.documentElement.style;
    root.setProperty("--seen-top", `${seen.offsetTop}px`);
    root.setProperty("--seen-height", `${seen.height}px`);
  };
  put();
  seen.addEventListener("resize", put);
  seen.addEventListener("scroll", put);
  onCleanup(() => {
    seen.removeEventListener("resize", put);
    seen.removeEventListener("scroll", put);
  });
}
