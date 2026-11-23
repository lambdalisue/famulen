/** Text as it is compared: full and half width, and upper and lower case, read the same. */
export const fold = (text: string) => text.normalize("NFKC").toLowerCase();

/** Whether every word of `words` is somewhere in `text`. */
export function hasWords(text: string, words: string): boolean {
  const hay = fold(text);
  return fold(words)
    .split(/\s+/)
    .filter(Boolean)
    .every((w) => hay.includes(w));
}

const names = new Intl.Collator("ja");

/**
 * The order of two names. Not `localeCompare` with the runtime's own locale:
 * the server and the browser may run in different ones, and a list the server
 * ordered otherwise is taken over row by row for the wrong records.
 */
export const byName = (a: string, b: string) => names.compare(a, b);
