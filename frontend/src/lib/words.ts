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
