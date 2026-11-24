/** The widths of `--phone` in app.css, for what a script decides. */
const NARROW = "(max-width: 700px)";

export const isNarrow = () => window.matchMedia(NARROW).matches;
