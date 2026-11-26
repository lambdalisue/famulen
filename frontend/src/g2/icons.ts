/**
 * The glasses' icons: Tabler's shapes redrawn on a 12×12 grid and drawn at
 * twice that, as the first-party icons are made of 2×2 dots. Images cannot
 * sit inside text, so icons are drawn into image boxes beside it.
 */

import type { Level } from "./page";

const GRIDS = {
  talk: [
    "............",
    "..########..",
    ".#........#.",
    ".#........#.",
    ".#........#.",
    ".#........#.",
    "..#.######..",
    ".##.........",
    ".#..........",
    "............",
    "............",
    "............",
  ],
  calendar: [
    "............",
    "...#....#...",
    ".##########.",
    ".#..#....#.#",
    ".##########.",
    ".#........#.",
    ".#........#.",
    ".#........#.",
    ".#........#.",
    ".##########.",
    "............",
    "............",
  ],
  tasks: [
    "............",
    "...#........",
    "#.#..#######",
    ".#..........",
    "............",
    "...#........",
    "#.#..#######",
    ".#..........",
    "............",
    "...#........",
    "#.#..#######",
    ".#..........",
  ],
  confirm: [
    "....####....",
    "..##....##..",
    ".#...##...#.",
    ".#...##...#.",
    "#....##....#",
    "#....##....#",
    "#....##....#",
    ".#........#.",
    ".#...##...#.",
    "..##....##..",
    "....####....",
    "............",
  ],
  memos: [
    "............",
    "..########..",
    "..#......#..",
    "..#.####.#..",
    "..#......#..",
    "..#.####.#..",
    "..#......#..",
    "..#.##...#..",
    "..#......#..",
    "..########..",
    "............",
    "............",
  ],
  mic: [
    "....####....",
    "...#....#...",
    "...#....#...",
    "...#....#...",
    ".#.#....#.#.",
    ".#..####..#.",
    "..#......#..",
    "...######...",
    ".....##.....",
    "....####....",
    "............",
    "............",
  ],
} as const;

export type IconName = keyof typeof GRIDS;

/** The grey each brightness is drawn in; the glasses keep 16 levels. */
const GREY: Record<Level, number> = { 1: 80, 2: 135, 3: 190, 4: 255 };

/** One thing drawn into an image: an icon, or a short word. */
export type Mark =
  | { icon: IconName; x: number; y: number; level: Level; scale?: number }
  | { word: string; x: number; y: number; level: Level }
  | { rect: [number, number, number, number]; level: Level };

/** A picture of `w`×`h` holding `marks`, as base64 PNG. */
export function picture(w: number, h: number, marks: Mark[]): string {
  const canvas = document.createElement("canvas");
  canvas.width = w;
  canvas.height = h;
  const pen = canvas.getContext("2d");
  if (!pen) throw new Error("this browser cannot draw");
  pen.fillStyle = "#000";
  pen.fillRect(0, 0, w, h);
  for (const m of marks) {
    const g = GREY[m.level];
    pen.fillStyle = `rgb(${g},${g},${g})`;
    if ("rect" in m) pen.fillRect(...m.rect);
    else if ("icon" in m) {
      const dot = m.scale ?? 2;
      for (const [row, line] of GRIDS[m.icon].entries()) {
        for (const [col, on] of [...line].entries()) if (on === "#") pen.fillRect(m.x + col * dot, m.y + row * dot, dot, dot);
      }
    } else {
      pen.font = "bold 20px sans-serif";
      pen.textBaseline = "middle";
      pen.fillText(m.word, m.x, m.y);
    }
  }
  return canvas.toDataURL("image/png").replace(/^data:image\/png;base64,/, "");
}
