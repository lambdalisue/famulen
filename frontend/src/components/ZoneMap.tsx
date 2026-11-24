import { For, Show, createSignal } from "solid-js";
import { LAND, ZONE_PLACES } from "~/lib/world";
import { nearestZone, onMap, zone } from "~/lib/zones";

const PLACES = Object.entries(ZONE_PLACES).map(([id, place]) => ({ id, at: onMap(place) }));

/**
 * The world with a dot for each time zone's main place. Pressing anywhere
 * picks the zone whose place is nearest.
 */
export default function ZoneMap(props: { value: string; onPick: (zone: string) => void }) {
  const [hover, setHover] = createSignal<string>();
  let svg: SVGSVGElement | undefined;

  const at = (e: MouseEvent): string | undefined => {
    const ctm = svg?.getScreenCTM();
    if (!svg || !ctm) return undefined;
    const p = new DOMPoint(e.clientX, e.clientY).matrixTransform(ctm.inverse());
    return nearestZone(p.x, p.y);
  };
  const chosen = () => ZONE_PLACES[props.value];

  return (
    <div class="zone-map">
      <svg
        ref={svg}
        viewBox="0 20 720 270"
        role="img"
        aria-label="タイムゾーンの地図"
        onMouseMove={(e) => setHover(at(e))}
        onMouseLeave={() => setHover(undefined)}
        onClick={(e) => {
          const zone = at(e);
          if (zone) props.onPick(zone);
        }}
      >
        <path class="land" d={LAND} />
        <For each={PLACES}>
          {(p) => <circle class="place" classList={{ near: p.id === hover() }} cx={p.at[0]} cy={p.at[1]} r={p.id === hover() ? 3 : 1.6} />}
        </For>
        <Show when={chosen()}>
          {(place) => {
            const xy = () => onMap(place());
            return (
              <g class="chosen">
                <circle cx={xy()[0]} cy={xy()[1]} r="9" class="halo" />
                <circle cx={xy()[0]} cy={xy()[1]} r="4" />
              </g>
            );
          }}
        </Show>
      </svg>
      <span class="sub zone-map-hint">
        {(() => {
          const id = hover();
          if (!id) return "地図を押すと、いちばん近いタイムゾーンを選ぶ";
          const z = zone(id);
          return `${z.id}　${z.name}（${z.offset}）`;
        })()}
      </span>
    </div>
  );
}
