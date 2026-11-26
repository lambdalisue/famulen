/**
 * What the glasses show, as boxes placed on the 576×288 screen, and how a
 * page reaches them: built once, then changed in place where only words or
 * pictures changed, as a rebuild flickers on the glasses.
 */

/** How bright text is, 1 the dimmest seen and 4 the brightest. */
export type Level = 1 | 2 | 3 | 4;

export const WIDTH = 576;
export const HEIGHT = 288;

export type TextBox = {
  name: string;
  x: number;
  y: number;
  w: number;
  h: number;
  text: string;
  level: Level;
  /** A frame around it, as bright as this. */
  border?: Level;
  padding?: number;
  /** The box that takes the owner's taps and swipes; one on each page. */
  capture?: boolean;
};

export type ImageBox = {
  name: string;
  x: number;
  y: number;
  w: number;
  h: number;
  /** A PNG of exactly w×h, base64 without a data: prefix. */
  png: string;
};

export type Page = { texts: TextBox[]; images: ImageBox[] };

/** What the glasses' firmware takes on one page. */
export const LIMITS = { texts: 8, images: 4, all: 12 } as const;

/** Why a page cannot be shown, or undefined when it can. */
export function invalid(page: Page): string | undefined {
  const { texts, images } = page;
  if (texts.length > LIMITS.texts) return `${texts.length} texts`;
  if (images.length > LIMITS.images) return `${images.length} images`;
  if (texts.length + images.length > LIMITS.all) return `${texts.length + images.length} boxes`;
  if (texts.filter((t) => t.capture).length !== 1) return "not exactly one box takes input";
  const names = [...texts, ...images].map((b) => b.name);
  if (new Set(names).size !== names.length) return "names repeat";
  if (names.some((n) => new TextEncoder().encode(n).length > 16)) return "a name is longer than 16 bytes";
  for (const i of images) {
    if (i.w < 20 || i.w > 288 || i.h < 20 || i.h > 144) return `image ${i.name} is ${i.w}×${i.h}`;
  }
  for (const b of [...texts, ...images]) {
    if (b.x < 0 || b.y < 0 || b.x + b.w > WIDTH || b.y + b.h > HEIGHT) return `${b.name} leaves the screen`;
  }
  return undefined;
}

/** The glasses' text brightness takes 0 to 4; a border takes 0 to 15. */
const BORDER: Record<Level, number> = { 1: 4, 2: 7, 3: 11, 4: 15 };

export type TextContainer = {
  xPosition: number;
  yPosition: number;
  width: number;
  height: number;
  containerID: number;
  containerName: string;
  content: string;
  textColor: number;
  borderWidth: number;
  borderColor: number;
  borderRadius: number;
  paddingLength: number;
  isEventCapture: number;
  zOrderIndex: number;
};

export type ImageContainer = {
  xPosition: number;
  yPosition: number;
  width: number;
  height: number;
  containerID: number;
  containerName: string;
  zOrderIndex: number;
};

export type Layout = { containerTotalNum: number; textObject: TextContainer[]; imageObject: ImageContainer[] };

/** The calls the glasses answer, as the Even App's bridge offers them. */
export interface Bridge {
  create(layout: Layout): Promise<boolean>;
  rebuild(layout: Layout): Promise<boolean>;
  text(id: number, name: string, content: string): Promise<boolean>;
  image(id: number, name: string, png: string): Promise<boolean>;
}

/** The containers a page is drawn with; later boxes sit in front. */
export function layoutOf(page: Page): Layout {
  const textObject = page.texts.map((t, i) => ({
    xPosition: t.x,
    yPosition: t.y,
    width: t.w,
    height: t.h,
    containerID: i + 1,
    containerName: t.name,
    content: t.text,
    textColor: t.level,
    borderWidth: t.border ? 1 : 0,
    borderColor: t.border ? BORDER[t.border] : 0,
    borderRadius: t.border ? 8 : 0,
    paddingLength: t.padding ?? 0,
    isEventCapture: t.capture ? 1 : 0,
    zOrderIndex: i + 1,
  }));
  const imageObject = page.images.map((m, i) => ({
    xPosition: m.x,
    yPosition: m.y,
    width: m.w,
    height: m.h,
    containerID: page.texts.length + i + 1,
    containerName: m.name,
    zOrderIndex: page.texts.length + i + 1,
  }));
  return { containerTotalNum: textObject.length + imageObject.length, textObject, imageObject };
}

/** A layout without what is drawn in it: two pages of the same shape differ only in words and pictures. */
function shapeOf(layout: Layout): string {
  return JSON.stringify({ ...layout, textObject: layout.textObject.map((t) => ({ ...t, content: "" })) });
}

/**
 * Shows pages one after another on the glasses. Calls go one at a time, as
 * the bridge cannot take two at once, and a page asked for while another is
 * still being sent replaces any that waits behind it.
 */
export class Painter {
  #bridge: Bridge;
  #shown: { layout: Layout; page: Page } | undefined;
  #started = false;
  #next: Page | undefined;
  #running: Promise<void> | undefined;
  /** How many times the glasses lost what they showed. */
  #forgotten = 0;

  constructor(bridge: Bridge) {
    this.#bridge = bridge;
  }

  show(page: Page): Promise<void> {
    const why = invalid(page);
    if (why) throw new Error(`the glasses cannot show this page: ${why}`);
    this.#next = page;
    this.#running ??= this.#drain();
    return this.#running;
  }

  /** Draw the page again from nothing, as the glasses lost it. */
  forget() {
    this.#shown = undefined;
    this.#forgotten++;
  }

  async #drain() {
    // Let go before returning, with nothing awaited between, so a page asked for meanwhile is not dropped.
    try {
      while (this.#next) {
        const page = this.#next;
        this.#next = undefined;
        // A call that fails leaves the page to be drawn whole, and the pages after it still go.
        await this.#paint(page).catch(() => {
          this.#shown = undefined;
        });
      }
    } finally {
      this.#running = undefined;
    }
  }

  async #paint(page: Page) {
    const forgotten = this.#forgotten;
    const layout = layoutOf(page);
    const before = this.#shown;
    // What the glasses show is not known after a call fails, so the next page is drawn whole.
    this.#shown = undefined;
    let ok = true;
    if (before && shapeOf(before.layout) === shapeOf(layout)) {
      for (const [i, t] of layout.textObject.entries()) {
        if (t.content !== before.layout.textObject[i].content) ok = (await this.#bridge.text(t.containerID, t.containerName, t.content)) && ok;
      }
      for (const [i, m] of page.images.entries()) {
        if (m.png !== before.page.images[i].png) ok = (await this.#bridge.image(layout.imageObject[i].containerID, m.name, m.png)) && ok;
      }
    } else {
      // The first page is made once; every later one is rebuilt, even when
      // making it failed, as making it again is refused.
      if (this.#started) ok = await this.#bridge.rebuild(layout);
      else {
        this.#started = true;
        ok = (await this.#bridge.create(layout)) || (await this.#bridge.rebuild(layout));
      }
      for (const [i, m] of page.images.entries()) ok = (await this.#bridge.image(layout.imageObject[i].containerID, m.name, m.png)) && ok;
    }
    // Lost while it was being drawn, the page is drawn whole the next time.
    if (ok && forgotten === this.#forgotten) this.#shown = { layout, page };
  }
}
