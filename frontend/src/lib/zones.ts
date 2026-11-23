import { ZONE_PLACES } from "./world";

/** One time zone as the owner picks it. */
export interface Zone {
  /** The IANA name, such as Asia/Tokyo. */
  id: string;
  /** Its generic name in Japanese, such as 日本時間, or empty. */
  name: string;
  /** Its offset from UTC now, such as UTC+9. */
  offset: string;
}

function part(zone: string, timeZoneName: "longGeneric" | "shortOffset"): string {
  try {
    return (
      new Intl.DateTimeFormat("ja-JP", { timeZone: zone, timeZoneName })
        .formatToParts(new Date())
        .find((p) => p.type === "timeZoneName")?.value ?? ""
    );
  } catch {
    return "";
  }
}

export function zone(id: string): Zone {
  const offset = part(id, "shortOffset").replace("GMT", "UTC");
  return { id, name: part(id, "longGeneric"), offset: offset === "UTC" ? "UTC+0" : offset };
}

/** Every zone the runtime knows, by name. */
export function allZones(): Zone[] {
  const ids = typeof Intl.supportedValuesOf === "function" ? Intl.supportedValuesOf("timeZone") : [];
  return ids.map(zone);
}

/** The zone this browser runs in. */
export function browserZone(): string {
  return Intl.DateTimeFormat().resolvedOptions().timeZone;
}

/** Where a place falls on the map's 720 x 360 equirectangular box. */
export function onMap([lat, lon]: [number, number]): [number, number] {
  return [(lon + 180) * 2, (90 - lat) * 2];
}

/** The zone whose main place lies nearest the point on the map. */
export function nearestZone(x: number, y: number): string | undefined {
  let best: string | undefined;
  let bestDistance = Number.POSITIVE_INFINITY;
  for (const [id, place] of Object.entries(ZONE_PLACES)) {
    const [px, py] = onMap(place);
    // The map wraps at the date line.
    const dx = Math.min(Math.abs(px - x), 720 - Math.abs(px - x));
    const distance = dx * dx + (py - y) * (py - y);
    if (distance < bestDistance) {
      bestDistance = distance;
      best = id;
    }
  }
  return best;
}
