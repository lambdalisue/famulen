/**
 * The glasses' screens, from what each shows to the boxes it is drawn with.
 *
 * Selection is shown by brightness alone, and a line's brightness is its
 * box's: lines of different brightness at one place are drawn as one box for
 * each brightness, each holding its own lines and blank lines for the rest.
 * Boxes never overlap otherwise, as nothing has a fill.
 */

import { type IconName, type Mark, picture } from "./icons";
import { HEIGHT, type ImageBox, type Level, type Page, type TextBox, WIDTH } from "./page";
import { clip, LINE, linesIn, measure, pad, wrap } from "./text";

export type Line = { text: string; level: Level };

/** A drawing function, so that screens can be built where nothing can draw. */
export type Draw = (w: number, h: number, marks: Mark[]) => string;

export const draw: Draw = picture;

/** What the top of every screen shows. */
export type Top = {
  /** "9:41  11/26 水" */
  clock: string;
  workgroup: string;
  confirms: number;
  tasks: number;
};

/** Boxes at one place, one for each brightness its lines have. */
export function stacked(
  name: string,
  at: { x: number; y: number; w: number; h: number; padding?: number; border?: Level; capture?: boolean },
  lines: Line[],
): TextBox[] {
  const levels = [...new Set(lines.filter((l) => l.text !== "").map((l) => l.level))].sort();
  if (levels.length === 0) levels.push(at.border ?? 1);
  return levels.map((level, i) => {
    const text = lines
      .map((l) => (l.level === level ? l.text : ""))
      .join("\n")
      .replace(/\n+$/, "");
    return {
      name: `${name}${level}`,
      x: at.x,
      y: at.y,
      w: at.w,
      h: at.h,
      text,
      level,
      padding: at.padding,
      border: i === 0 ? at.border : undefined,
      capture: i === 0 ? at.capture : undefined,
    };
  });
}

function top(t: Top, art: Draw): { texts: TextBox[]; images: ImageBox[] } {
  const marks: Mark[] = [
    { icon: "confirm", x: 0, y: 0, level: t.confirms > 0 ? 4 : 2 },
    { word: String(t.confirms), x: 30, y: 12, level: t.confirms > 0 ? 4 : 2 },
    { icon: "tasks", x: 62, y: 0, level: 3 },
    { word: String(t.tasks), x: 92, y: 12, level: 3 },
  ];
  return {
    texts: [
      { name: "clock", x: 14, y: 6, w: 190, h: 32, padding: 0, text: t.clock, level: 3 },
      { name: "group", x: 206, y: 6, w: 230, h: 32, padding: 0, text: clip(t.workgroup, 220), level: 2 },
    ],
    images: [{ name: "status", x: 446, y: 9, w: 120, h: 24, png: art(120, 24, marks) }],
  };
}

function page(t: Top, art: Draw, texts: TextBox[], images: ImageBox[] = []): Page {
  const head = top(t, art);
  // The box that takes input comes first among those drawn below the top.
  return { texts: [...head.texts, ...texts], images: [...head.images, ...images] };
}

/** The part of `items` a box of `room` lines shows, keeping `selected` in it. */
export function visible<T>(items: T[], selected: number, room: number): { start: number; shown: T[] } {
  if (items.length <= room) return { start: 0, shown: items };
  const start = Math.min(Math.max(0, selected - Math.floor(room / 2)), items.length - room);
  return { start, shown: items.slice(start, start + room) };
}

/** A thin line at the right of a box, as long and as far down as what is shown of the whole. */
function scrollbar(name: string, x: number, y: number, h: number, at: number, of: number, art: Draw): ImageBox {
  const length = Math.max(8, Math.round(h / of));
  const from = of <= 1 ? 0 : Math.round(((h - length) * at) / (of - 1));
  return { name, x, y, w: 20, h, png: art(20, h, [{ rect: [16, from, 2, length], level: 3 }]) };
}


/** The conversation, which the glasses open on. */
export type Talk = {
  /** The lines of what is shown now, already broken. */
  lines: Line[];
  /** Which page of the conversation, and of how many. */
  at: number;
  of: number;
};

const TALK = { x: 20, y: 44, w: 536, h: 238, padding: 12 };
/** How wide a line of the conversation may be, leaving room for its scroll line. */
export const TALK_WIDTH = TALK.w - 2 * TALK.padding - 28;
export const TALK_LINES = linesIn(TALK.h, TALK.padding);

export function talkScreen(t: Top, talk: Talk, art: Draw = draw): Page {
  const texts = stacked("talk", { ...TALK, border: 3, capture: true }, talk.lines.slice(0, TALK_LINES));
  const images = talk.of > 1 ? [scrollbar("bar", TALK.x + TALK.w - 26, TALK.y + 12, 144, talk.at, talk.of, art)] : [];
  return page(t, art, texts, images);
}


/** The menu: a narrow list on the left, what the chosen item holds on the right. */
export type MenuItem = { icon: IconName; label: string; count?: number };

export type Card = { icon: IconName; title: string; note: string; lines: Line[] };

const MENU = { x: 10, y: 44, w: 206, h: 232, padding: 28 };
const CARD = { x: 222, y: 44, w: 344, h: 234, padding: 12 };
export const CARD_WIDTH = CARD.w - 2 * CARD.padding;
export const CARD_LINES = linesIn(CARD.h, CARD.padding) - 1;

export function menuScreen(t: Top, items: MenuItem[], selected: number, card: Card, art: Draw = draw): Page {
  const names = items.map((m, i) => ({
    text: ` ${m.label}${m.count ? `   ${m.count}` : ""}`,
    level: (i === selected ? 4 : 2) as Level,
  }));
  const column: Mark[] = items.map((m, i) => ({ icon: m.icon, x: 0, y: i * LINE + 1, level: i === selected ? 4 : 2 }));
  const head = { text: `${pad("", 30)}${card.title}${card.note ? `   ${card.note}` : ""}`, level: 3 as Level };
  return page(
    t,
    art,
    [
      ...stacked("menu", { ...MENU, border: 2, capture: true }, names),
      ...stacked("card", { ...CARD, border: 2 }, [head, ...card.lines.slice(0, CARD_LINES)]),
    ],
    [
      { name: "column", x: MENU.x + 6, y: MENU.y + MENU.padding, w: 24, h: items.length * LINE, png: art(24, items.length * LINE, column) },
      { name: "cardicon", x: CARD.x + CARD.padding, y: CARD.y + CARD.padding + 1, w: 24, h: 24, png: art(24, 24, [{ icon: card.icon, x: 0, y: 0, level: 3 }]) },
    ],
  );
}


/** A list: today's events, tasks, what waits for an answer, memos. */
export type Row = { text: string; level: Level };

const HEAD = { x: 56, y: 48, w: 500, h: 32, padding: 0 };
const ROWS = { x: 20, y: 80, w: 536, h: 204, padding: 4 };
export const ROW_WIDTH = ROWS.w - 2 * ROWS.padding - 16;

export function listScreen(
  t: Top,
  head: { icon: IconName; title: string; note: string },
  rows: Row[],
  selected: number,
  below: Line[] = [],
  art: Draw = draw,
): Page {
  const room = linesIn(ROWS.h, ROWS.padding) - (below.length ? below.length + 1 : 0);
  const { start, shown } = visible(rows, selected, room);
  const lines: Line[] = shown.map((r, i) => ({ text: ` ${r.text}`, level: start + i === selected ? 4 : r.level }));
  if (rows.length === 0) lines.push({ text: " ない", level: 2 });
  if (below.length) lines.push({ text: "", level: 1 }, ...below.map((l) => ({ ...l, text: ` ${l.text}` })));
  const titleWidth = measure(head.title) + 2 * HEAD.padding;
  return page(
    t,
    art,
    [
      ...stacked("rows", { ...ROWS, capture: true }, lines),
      { name: "title", ...HEAD, w: Math.ceil(titleWidth) + 4, text: head.title, level: 3 },
      { name: "note", ...HEAD, x: HEAD.x + Math.ceil(titleWidth) + 16, w: HEAD.w - Math.ceil(titleWidth) - 16, text: head.note, level: 2 },
    ],
    [{ name: "headicon", x: 22, y: 50, w: 24, h: 24, png: art(24, 24, [{ icon: head.icon, x: 0, y: 0, level: 3 }]) }],
  );
}


/** One item in a frame, and what can be done with it below. */
export type Held = {
  icon: IconName;
  title: string;
  note: string;
  /** What it holds, already broken; shown whole, a page at a time. */
  body: string[];
  at: number;
  of: number;
};

export const HELD_WIDTH = WIDTH - 40 - 54 - 24;

/** The most answers shown at once; more are scrolled through. */
const CHOICES_SHOWN = 4;

/** The tallest the frame may be while the answers shown still fit below it. */
function frameRoom(choices: number): number {
  return HEIGHT - 44 - 6 - (Math.min(choices, CHOICES_SHOWN) * LINE + 8);
}

/** How many lines of an item's text a page shows above `choices` answers, its page mark left room. */
export function heldLines(choices: number): number {
  return Math.max(1, linesIn(frameRoom(choices), 10) - 3);
}

export function heldScreen(t: Top, held: Held, choices: string[], selected: number, art: Draw = draw): Page {
  const frameH = (2 + held.body.length + (held.of > 1 ? 1 : 0)) * LINE + 20;
  const frame = { x: 20, y: 44, w: 536, h: Math.min(frameH, frameRoom(choices.length)), padding: 10 };
  const indent = pad("", 42);
  const body: Line[] = [
    { text: "", level: 3 },
    ...held.body.map((text) => ({ text: `${indent}${text}`, level: 3 as Level })),
    ...(held.of > 1 ? [{ text: `${indent}(${held.at + 1}/${held.of})`, level: 1 as Level }] : []),
  ];
  const titleWidth = measure(held.title);
  const { start, shown } = visible(choices, selected, CHOICES_SHOWN);
  const options: Line[] = shown.map((c, i) => ({ text: ` ${c}`, level: start + i === selected ? 4 : 2 }));
  const optionsY = frame.y + frame.h + 6;
  return page(
    t,
    art,
    [
      ...stacked("opts", { x: 20, y: optionsY, w: 536, h: HEIGHT - optionsY, padding: 4, capture: true }, options),
      ...stacked("frame", { ...frame, border: 3 }, body),
      { name: "heldtitle", x: 62, y: 54, w: Math.min(494, Math.ceil(titleWidth) + 28), h: 32, padding: 0, text: held.title, level: 4 },
      { name: "heldnote", x: 62 + Math.ceil(titleWidth) + 28, y: 54, w: Math.max(40, 494 - Math.ceil(titleWidth) - 28), h: 32, padding: 0, text: clip(held.note, Math.max(20, 470 - titleWidth)), level: 2 },
    ],
    [{ name: "heldicon", x: 32, y: 56, w: 24, h: 24, png: art(24, 24, [{ icon: held.icon, x: 0, y: 0, level: 3 }]) }],
  );
}


/** Listening: what was said before, dim, and what is heard now in a box at the bottom. */
export const HEARD = { x: 140, y: 160, w: 344, h: 120, padding: 10 };
export const HEARD_WIDTH = HEARD.w - 2 * HEARD.padding;

export function listenScreen(t: Top, before: Line[], about: string | undefined, heard: string, art: Draw = draw): Page {
  const lines: Line[] = [];
  if (about) lines.push({ text: clip(about, HEARD_WIDTH), level: 2 });
  const room = linesIn(HEARD.h, HEARD.padding) - lines.length;
  const said = wrap(heard, HEARD_WIDTH);
  lines.push(...said.slice(Math.max(0, said.length - room)).map((text) => ({ text, level: 4 as Level })));
  return page(
    t,
    art,
    [
      ...stacked("before", { x: 20, y: 44, w: 536, h: 108, padding: 12, border: 1 }, before.slice(0, 3).map((l) => ({ ...l, level: 1 }))),
      { name: "listen", x: 88, y: 160, w: 400, h: 120, padding: 0, text: "", level: 4, border: 4 },
      ...stacked("heard", { ...HEARD, capture: true }, lines),
    ],
    [{ name: "mic", x: 100, y: 172, w: 32, h: 32, png: art(32, 32, [{ icon: "mic", x: 4, y: 4, level: 4 }]) }],
  );
}

/** The dots that step while something is being heard or thought: . → .. → ... */
export function dots(step: number): string {
  return ".".repeat((step % 3) + 1);
}
