/**
 * Laying words out for the glasses' one font, which cannot be measured from
 * the page: widths are estimated, and lines are broken here so that a page
 * knows how many lines it holds.
 */

/** About how wide the glasses draw a full-width character, in pixels. */
export const EM = 20;
/** The height of one line. */
export const LINE = 28;
/** About how wide a plain space is drawn. */
const SPACE = 6;

/** Characters that may not begin a line, as Japanese is set. */
const NO_START = new Set([..."、。，．・：；？！ー）」』】〕〉》’”ぁぃぅぇぉっゃゅょゎァィゥェォッャュョヮヵヶ々…‥"]);

const graphemes = new Intl.Segmenter("ja", { granularity: "grapheme" });

/** The characters of `text` as they are seen, so none is cut in two. */
export function chars(text: string): string[] {
  return Array.from(graphemes.segment(text), (s) => s.segment);
}

/** About how wide `ch` is drawn. */
export function widthOf(ch: string): number {
  const c = ch.codePointAt(0) ?? 0;
  if (c < 0x20) return 0;
  if (ch === " ") return SPACE;
  if (c < 0x7f) return /[ilj.,:;'|!1]/.test(ch) ? EM * 0.25 : /[mwMW@]/.test(ch) ? EM * 0.75 : EM * 0.5;
  if (c < 0x2000) return EM * 0.6;
  if (c >= 0xff61 && c <= 0xff9f) return EM * 0.55;
  return EM;
}

/** About how wide `text` is drawn. */
export function measure(text: string): number {
  return chars(text).reduce((sum, ch) => sum + widthOf(ch), 0);
}

/** `text` broken into lines no wider than `width`, its own line breaks kept. */
export function wrap(text: string, width: number): string[] {
  const lines: string[] = [];
  for (const paragraph of text.replace(/\r\n?/g, "\n").split("\n")) {
    let line: string[] = [];
    let used = 0;
    for (const ch of chars(paragraph)) {
      const w = widthOf(ch);
      // A character that may not begin a line hangs past the edge, but by no more than a little.
      const hangs = NO_START.has(ch) && used + w <= width + EM;
      if (used + w > width && line.length > 0 && !hangs) {
        lines.push(line.join("").trimEnd());
        line = [];
        used = 0;
        if (ch === " ") continue;
      }
      line.push(ch);
      used += w;
    }
    lines.push(line.join("").trimEnd());
  }
  return lines;
}

/** `text` cut to fit `width` on one line, ending with … when it was cut. */
export function clip(text: string, width: number): string {
  const one = text.replace(/\s+/g, " ").trim();
  if (measure(one) <= width) return one;
  let out = "";
  let used = widthOf("…");
  for (const ch of chars(one)) {
    used += widthOf(ch);
    if (used > width) break;
    out += ch;
  }
  return `${out.trimEnd()}…`;
}

/** `text` followed by spaces up to about `width`, so what comes after it lines up. */
export function pad(text: string, width: number): string {
  const missing = Math.max(SPACE, width - measure(text));
  return text + "\u3000".repeat(Math.floor(missing / EM)) + " ".repeat(Math.round((missing % EM) / SPACE));
}

/** How many lines a box of `height` holds, with `padding` inside it. */
export function linesIn(height: number, padding = 0): number {
  return Math.max(1, Math.floor((height - 2 * padding) / LINE));
}
