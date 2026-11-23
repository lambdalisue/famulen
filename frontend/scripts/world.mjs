// Writes src/lib/world.ts. It needs packages the console does not, so run it
// from a scratch directory that has them:
//
//   npm install d3-geo@3.1.2 topojson-client@3.1.0 world-atlas@2.0.2
//   node <this file> <console>/src/lib/world.ts
//
// zone.tab comes from tzdata (/usr/share/zoneinfo/zone.tab on most systems).
// Write the land of the world as one SVG path in an equirectangular box of
// 720 x 360 (x = (lon + 180) * 2, y = (90 - lat) * 2), and every zone of
// zone.tab with its coordinates.
import { readFileSync, writeFileSync } from "node:fs";
import { geoEquirectangular, geoPath } from "d3-geo";
import { feature } from "topojson-client";

const topo = JSON.parse(readFileSync("node_modules/world-atlas/land-110m.json", "utf8"));
const land = feature(topo, topo.objects.land);
const projection = geoEquirectangular().scale(360 / (2 * Math.PI) * 2).translate([360, 180]);
const path = geoPath(projection).digits(1)(land);

const zones = {};
for (const line of readFileSync("/usr/share/zoneinfo/zone.tab", "utf8").split("\n")) {
  if (!line || line.startsWith("#")) continue;
  const [, coord, name] = line.split("\t");
  const m = coord.match(/^([+-])(\d{2})(\d{2})(\d{2})?([+-])(\d{3})(\d{2})(\d{2})?$/);
  if (!m) continue;
  const lat = (m[1] === "-" ? -1 : 1) * (+m[2] + +m[3] / 60 + (+m[4] || 0) / 3600);
  const lon = (m[5] === "-" ? -1 : 1) * (+m[6] + +m[7] / 60 + (+m[8] || 0) / 3600);
  zones[name] = [Math.round(lat * 10) / 10, Math.round(lon * 10) / 10];
}
const out = process.argv[2];
writeFileSync(
  out,
  `// Written by scripts/world.mjs from Natural Earth's land (world-atlas, land-110m) and tzdata's
// zone.tab: the land as one path in an equirectangular 720 x 360 box, and
// each time zone's main place as [latitude, longitude].

export const LAND = ${JSON.stringify(path)};

export const ZONE_PLACES: Record<string, [number, number]> = ${JSON.stringify(zones)};
`,
);
console.log(path.length, Object.keys(zones).length);
