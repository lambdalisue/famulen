/*
 * What this browser keeps for one viewer, such as a width or a layout.
 * Storage can be missing or refuse to answer, so the page works without it.
 */

export function kept(key: string): string | undefined {
  try {
    return localStorage.getItem(key) ?? undefined;
  } catch {
    return undefined;
  }
}

/** Keep a value, or forget it when it is undefined. */
export function keep(key: string, value: string | undefined) {
  try {
    if (value === undefined) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    // The choice lasts until the page is left.
  }
}
