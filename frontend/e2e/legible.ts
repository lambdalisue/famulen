import type { Page } from "@playwright/test";

/**
 * Every text on the page that stands out too little from what is behind it,
 * by WCAG AA, and every field whose edge does.
 */
export function faint(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const pen = Object.assign(document.createElement("canvas"), { width: 1, height: 1 }).getContext("2d", { willReadFrequently: true });
    if (!pen) throw new Error("no canvas");
    type Color = [number, number, number, number];
    const seen = new Map<string, Color>();
    const color = (css: string): Color => {
      const known = seen.get(css);
      if (known) return known;
      pen.clearRect(0, 0, 1, 1);
      pen.fillStyle = css;
      pen.fillRect(0, 0, 1, 1);
      const [r, g, b, a] = pen.getImageData(0, 0, 1, 1).data;
      const made: Color = [r, g, b, a / 255];
      seen.set(css, made);
      return made;
    };
    const over = (a: Color, b: Color): Color => [a[0] * a[3] + b[0] * (1 - a[3]), a[1] * a[3] + b[1] * (1 - a[3]), a[2] * a[3] + b[2] * (1 - a[3]), 1];
    const light = (c: Color) => {
      const [r, g, b] = c.slice(0, 3).map((v) => (v / 255 <= 0.04045 ? v / 255 / 12.92 : ((v / 255 + 0.055) / 1.055) ** 2.4));
      return 0.2126 * r + 0.7152 * g + 0.0722 * b;
    };
    const ratio = (a: Color, b: Color) => {
      const [x, y] = [light(a), light(b)].sort((p, q) => q - p);
      return (x + 0.05) / (y + 0.05);
    };
    const behind = (from: Element | null): Color => {
      const layers: Color[] = [];
      for (let e = from; e; e = e.parentElement) {
        const c = color(getComputedStyle(e).backgroundColor);
        if (c[3]) layers.push(c);
        if (c[3] === 1) break;
      }
      return layers.reduceRight((b, c) => over(c, b), color(getComputedStyle(document.body).backgroundColor));
    };
    const found = new Set<string>();
    const walk = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let n = walk.nextNode(); n; n = walk.nextNode()) {
      const el = n.parentElement;
      const text = n.textContent?.trim();
      // What cannot be pressed is left out of AA.
      if (!el || !text || el.closest("script, style, [hidden], :disabled, [data-disabled], option, .sr, .markdown-body pre") || !el.checkVisibility()) continue;
      const style = getComputedStyle(el);
      if (style.fontSize === "0px") continue;
      const back = behind(el);
      const ink: Color = [...color(style.color)];
      for (let e: Element | null = el; e; e = e.parentElement) ink[3] *= Number(getComputedStyle(e).opacity);
      const got = ratio(over(ink, back), back);
      if (got < (Number.parseFloat(style.fontSize) >= 24 ? 3 : 4.5)) found.add(`${got.toFixed(2)} "${text.slice(0, 16)}" .${el.className}`);
    }
    for (const el of document.querySelectorAll(".input, .check-box")) {
      if (!el.checkVisibility() || el.matches(":disabled, :checked")) continue;
      const got = ratio(color(getComputedStyle(el).borderTopColor), behind(el.parentElement));
      if (got < 3) found.add(`edge ${got.toFixed(2)} .${el.className}`);
    }
    return [...found];
  });
}
