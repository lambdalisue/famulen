export const COLUMNS = 12;
export const LEAST_CELLS = 3;
export const MOST_ROWS = 24;
/** As far down as a tile can start. */
const LAST_ROW = 200;

/** Where a tile is on the grid, in cells. */
export type Place = { x: number; y: number; width: number; height: number };
export type Placed = Place & { kind: string };

const between = (least: number, v: number, most: number) => Math.max(least, Math.min(most, v));

/** A place moved, then kept within the grid. */
export function movedTo(p: Place, x: number, y: number): Place {
  return { ...p, x: between(0, x, COLUMNS - p.width), y: between(0, y, LAST_ROW) };
}

/** A place made another size from its top left, kept within the grid. */
export function sizedTo(p: Place, width: number, height: number): Place {
  return { ...p, width: between(LEAST_CELLS, width, COLUMNS - p.x), height: between(LEAST_CELLS, height, MOST_ROWS) };
}

function overlaps(a: Place, b: Place): boolean {
  return a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;
}

/** Tiles in the order they are read: from the top, then from the left. */
export function inOrder<T extends Place>(tiles: readonly T[]): T[] {
  return [...tiles].sort((a, b) => a.y - b.y || a.x - b.x);
}

/**
 * Tiles with none over another: the one at `held` stays where it is, and one
 * it lies over goes below it. What nothing lies over stays where it was put.
 * Each keeps its place in the list.
 */
export function settled(tiles: readonly Placed[], held = -1): Placed[] {
  const down = new Map<Placed, Placed>();
  const first = tiles[held];
  for (const tile of first ? [first, ...inOrder(tiles).filter((t) => t !== first)] : inOrder(tiles)) {
    let at = tile;
    for (;;) {
      const under = [...down.values()].filter((o) => overlaps(o, at));
      if (under.length === 0) break;
      at = { ...at, y: Math.max(...under.map((o) => o.y + o.height)) };
    }
    down.set(tile, at);
  }
  return tiles.map((t) => down.get(t) ?? t);
}

/** The row below every tile. */
export function below(tiles: readonly Place[]): number {
  return Math.max(0, ...tiles.map((t) => t.y + t.height));
}
