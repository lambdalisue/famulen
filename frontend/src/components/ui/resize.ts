import { createSignal, onMount } from "solid-js";
import { keep, kept } from "~/lib/storage";

const STEP = 24;

/**
 * A width its owner drags, or moves with the arrow keys, kept in this browser
 * under `key`. `from` reads a width off the pointer; `toward` is the arrow
 * that widens; `now` is how wide it is when its width is left to the layout,
 * which a `width` of 0 does.
 */
export function createResizer(of: { key: string; width: number; least: number; most: () => number; from: (e: PointerEvent) => number; toward: "ArrowLeft" | "ArrowRight"; now?: () => number }) {
  const clamp = (w: number) => Math.round(Math.min(Math.max(of.least, of.most()), Math.max(of.least, w)));
  const [width, setWidth] = createSignal(of.width);
  onMount(() => {
    const was = Number(kept(of.key));
    if (was > 0) setWidth(clamp(was));
  });
  const set = (w: number) => {
    setWidth(w);
    keep(of.key, w === of.width ? undefined : String(w));
  };
  let dragging = false;
  const drop = (e: PointerEvent & { currentTarget: HTMLElement }) => {
    if (!dragging) return;
    dragging = false;
    e.currentTarget.releasePointerCapture(e.pointerId);
    set(width());
  };
  return {
    width,
    reset: () => set(of.width),
    changed: () => width() !== of.width,
    /** What the separator that is dragged carries. */
    separator: {
      role: "separator" as const,
      tabindex: 0,
      "aria-orientation": "vertical" as const,
      "aria-label": "幅を変える",
      "aria-valuemin": of.least,
      onPointerDown: (e: PointerEvent & { currentTarget: HTMLElement }) => {
        e.preventDefault();
        e.currentTarget.setPointerCapture(e.pointerId);
        dragging = true;
      },
      onPointerMove: (e: PointerEvent) => {
        if (dragging) setWidth(clamp(of.from(e)));
      },
      onPointerUp: drop,
      onPointerCancel: drop,
      onDblClick: () => set(of.width),
      onKeyDown: (e: KeyboardEvent) => {
        if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
        e.preventDefault();
        set(clamp((of.now?.() ?? width()) + (e.key === of.toward ? STEP : -STEP)));
      },
    },
  };
}
