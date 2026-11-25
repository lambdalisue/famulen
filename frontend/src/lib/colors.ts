/** A color the owner chose for a light ground, as it is drawn in either theme; the accent where none was chosen. */
export const own = (color: string) => (color ? `light-dark(${color}, color-mix(in oklab, ${color} 62%, white))` : "var(--accent)");
