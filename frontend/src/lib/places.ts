/** `path` with `~` read from `home` and no trailing slash, or undefined when it is not whole. */
export function wholePath(path: string, home: string | undefined): string | undefined {
  const p = path.trim();
  const full = p === "~" ? home : p.startsWith("~/") ? (home ? `${home.replace(/\/$/, "")}/${p.slice(2)}` : undefined) : p;
  if (!full?.startsWith("/")) return undefined;
  return full.length > 1 ? full.replace(/\/+$/, "") : full;
}

/** Whether `path` is `above` or lies under it, compared by whole folder names. */
export function isUnder(path: string, above: string): boolean {
  return path === above || path.startsWith(above === "/" ? "/" : `${above}/`);
}

/** The place among `places` that `path` is, or lies under. */
export function coveringPlace(path: string, places: readonly string[]): string | undefined {
  return places.find((p) => isUnder(path, p));
}
