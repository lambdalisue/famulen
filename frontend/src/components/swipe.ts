import { onCleanup, onMount } from "solid-js";
import { choosing } from "~/components/hold";

/** How far a finger goes before it is told whether it swipes or scrolls. */
const SLOP = 10;
/** How much more to the side than up or down a finger goes to swipe. */
const SIDEWAYS = 1.2;
/** How far across, as a part of the width, or how fast in px per ms, a swipe goes to go back. */
const FAR = 0.35;
const FAST = 0.4;
/** The strip at the left edge where the browser's own swipe back begins. */
const EDGE = 24;
/** How long a finger rests before it moves for the browser to take it as choosing text. */
const PRESS = 400;
/** How far the view beneath starts to the left, as a part of the width, and comes in with the swipe. */
const BENEATH = 0.3;
const SETTLE = 180;

/** What is written in, where a finger moves the caret and chooses text, and the box, which stays as the rest goes. */
const STAYS = "input, textarea, select, [contenteditable], .composer";

/** Whether something under the finger would scroll toward its start, as a finger going right scrolls it. */
function scrollsBack(from: EventTarget | null, until: HTMLElement) {
  for (let at = from instanceof Element ? from : null; at && until.contains(at); at = at.parentElement) {
    if (at.scrollWidth > at.clientWidth + 1 && /auto|scroll/.test(getComputedStyle(at).overflowX) && at.scrollLeft > 0) return true;
  }
  return false;
}

/** Whether the page was put on the home screen of iOS, where the browser has no swipe of its own. */
const standalone = () => (navigator as Navigator & { standalone?: boolean }).standalone === true;

/**
 * A swipe to the right over what is opened from a list, under the finger,
 * with the list coming in beneath it. Let go far or fast enough, it goes
 * back; let go short of that, or brought back, it stays.
 */
export function createSwipeBack(of: {
  /** Where a swipe begins, and what holds what moves. */
  area: () => HTMLElement | undefined;
  /** Whether there is anything to go back from. */
  can: () => boolean;
  /** What is open, to tell that another was opened while the swipe went on. */
  open: () => unknown;
  /** What moves with the finger. */
  moving: () => HTMLElement[];
  /** Show the list beneath, or stop showing it, and return it. */
  beneath: (shown: boolean) => HTMLElement | undefined;
  back: () => void;
}) {
  type Swipe = { id: number; x: number; y: number; at: number; on: boolean; last: number; when: number; speed: number; width: number; under?: HTMLElement; from: unknown };
  let swipe: Swipe | undefined;
  let settling = false;

  const put = (s: Swipe, by: number) => {
    const across = Math.max(0, Math.min(by, s.width));
    for (const el of of.moving()) el.style.transform = `translateX(${across}px)`;
    if (s.under) s.under.style.transform = `translateX(${(across / s.width - 1) * BENEATH * s.width}px)`;
  };
  const clear = (s: Swipe) => {
    for (const el of of.moving()) el.style.removeProperty("transform");
    s.under?.style.removeProperty("transform");
  };
  const settle = async (s: Swipe, by: number, going: boolean) => {
    settling = true;
    const to = going ? s.width : 0;
    const motion = { duration: SETTLE, easing: "ease-out", fill: "forwards" as const };
    const runs = of.moving().map((el) => el.animate([{ transform: `translateX(${by}px)` }, { transform: `translateX(${to}px)` }], motion));
    const from = (by / s.width - 1) * BENEATH * s.width;
    const under = going ? 0 : -BENEATH * s.width;
    if (s.under) runs.push(s.under.animate([{ transform: `translateX(${from}px)` }, { transform: `translateX(${under}px)` }], motion));
    await Promise.all(runs.map((r) => r.finished.catch(() => {})));
    clear(s);
    // What came meanwhile, as a notice opening another, is not gone back from.
    if (going && of.can() && of.open() === s.from) of.back();
    for (const r of runs) r.cancel();
    of.area()?.classList.remove("swiping");
    of.beneath(false);
    settling = false;
  };

  const start = (e: TouchEvent) => {
    const t = e.touches[0];
    const area = of.area();
    if (swipe?.on) {
      // Another finger leaves a swipe under way to the first.
      if (e.touches.length > 1) return;
      // One finger alone: the last swipe never heard it lift, as what it began on went away, and is put back.
      void settle(swipe, Math.max(0, Math.min(swipe.last - swipe.x, swipe.width)), false);
    }
    swipe = undefined;
    if (settling || e.touches.length !== 1 || !t || !area || !of.can()) return;
    if (e.target instanceof Element && e.target.closest(STAYS)) return;
    if (choosing(e.target)) return;
    if (!standalone() && t.clientX < EDGE) return;
    swipe = { id: t.identifier, x: t.clientX, y: t.clientY, at: e.timeStamp, on: false, last: t.clientX, when: e.timeStamp, speed: 0, width: area.clientWidth, from: of.open() };
  };
  const move = (e: TouchEvent) => {
    const s = swipe;
    const t = s && [...e.changedTouches].find((c) => c.identifier === s.id);
    const area = of.area();
    if (!s || !t || !area) return;
    const [dx, dy] = [t.clientX - s.x, t.clientY - s.y];
    if (!s.on) {
      if (Math.abs(dx) < SLOP && Math.abs(dy) < SLOP) return;
      // A pull of the sheet may have taken the finger since it came down.
      if (!of.can() || dx <= 0 || Math.abs(dx) < Math.abs(dy) * SIDEWAYS || e.timeStamp - s.at > PRESS || !e.cancelable || scrollsBack(e.target, area)) {
        swipe = undefined;
        return;
      }
      s.on = true;
      area.classList.add("swiping");
      s.under = of.beneath(true);
    }
    e.preventDefault();
    const passed = e.timeStamp - s.when;
    if (passed > 0) s.speed = (t.clientX - s.last) / passed;
    s.last = t.clientX;
    s.when = e.timeStamp;
    put(s, dx);
  };
  const end = (e: TouchEvent, cancelled: boolean) => {
    const s = swipe;
    const t = s && [...e.changedTouches].find((c) => c.identifier === s.id);
    if (!s || !t) return;
    swipe = undefined;
    if (!s.on) return;
    const by = Math.max(0, Math.min(t.clientX - s.x, s.width));
    const going = !cancelled && s.speed >= 0 && (by > s.width * FAR || s.speed > FAST);
    void settle(s, by, going);
  };
  const lift = (e: TouchEvent) => end(e, false);
  const cancel = (e: TouchEvent) => end(e, true);

  onMount(() => {
    const area = of.area();
    if (!area) return;
    area.addEventListener("touchstart", start, { passive: true });
    // Not passive, so that a swipe keeps what is under it from scrolling.
    area.addEventListener("touchmove", move, { passive: false });
    area.addEventListener("touchend", lift);
    area.addEventListener("touchcancel", cancel);
    onCleanup(() => {
      area.removeEventListener("touchstart", start);
      area.removeEventListener("touchmove", move);
      area.removeEventListener("touchend", lift);
      area.removeEventListener("touchcancel", cancel);
    });
  });
  /** Whether a swipe is under way, for what else a finger might start. */
  return { swiping: () => swipe?.on === true || settling };
}
