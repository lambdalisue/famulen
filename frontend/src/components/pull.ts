import { settledChat } from "~/lib/chatHistory";
import { isNarrow } from "~/lib/viewport";

/** How far a finger goes before it pulls rather than taps. */
const SLOP = 6;
/** How far, or how fast in px per ms, a pull goes to open or close. */
const FAR = 90;
const FAST = 0.5;

/** How much more to the right than down a finger goes on the head for it to be a swipe back, as the swipe takes it. */
const SIDEWAYS = 1.2;

const HANDLES = ".m-peek, .m-sheetbar, .talk-head";

/**
 * What a sheet carries so that it is pulled up by its peek and down by its
 * bar, under the finger, in the narrow layout.
 */
export function createPull(of: {
  sheet: () => HTMLElement | undefined;
  open: () => boolean;
  setOpen: (open: boolean) => void;
  /** Whether another movement of the finger has it already. */
  taken: () => boolean;
}) {
  let start: { id: number; x: number; y: number; across: boolean; open: boolean; at: number; last: number; speed: number } | undefined;
  let pulling = false;
  /** How far below its open place the sheet is. */
  let offset = 0;

  const put = (y: number) => {
    offset = Math.max(0, y);
    of.sheet()?.style.setProperty("transform", `translateY(${offset}px)`);
  };
  const settle = async (open: boolean) => {
    const sheet = of.sheet();
    try {
      if (!sheet) return;
      const to = open ? 0 : sheet.offsetHeight;
      sheet.style.removeProperty("transform");
      await sheet
        .animate([{ transform: `translateY(${offset}px)` }, { transform: `translateY(${to}px)` }], {
          duration: 160,
          easing: "ease-out",
        })
        .finished.catch(() => {});
      of.setOpen(open);
    } finally {
      document.body.classList.remove("chat-moving");
      settledChat();
    }
  };
  const drop = (e: PointerEvent, cancelled: boolean) => {
    if (!start || e.pointerId !== start.id) return;
    const { open, speed, y } = start;
    start = undefined;
    if (!pulling) return;
    pulling = false;
    const by = e.clientY - y;
    const enough = !cancelled && (open ? by > FAR || speed > FAST : by < -FAR || speed < -FAST);
    void settle(enough ? !open : open);
  };

  return {
    onPointerDown: (e: PointerEvent) => {
      if (!isNarrow() || !e.isPrimary || !(e.target instanceof Element) || !e.target.closest(HANDLES)) return;
      start = { id: e.pointerId, x: e.clientX, y: e.clientY, across: !!e.target.closest(".talk-head"), open: of.open(), at: e.timeStamp, last: e.clientY, speed: 0 };
    },
    onPointerMove: (e: PointerEvent & { currentTarget: HTMLElement }) => {
      if (!start || e.pointerId !== start.id) return;
      const by = e.clientY - start.y;
      if (!pulling) {
        if (Math.abs(by) < SLOP || (start.open ? by < 0 : by > 0)) return;
        // Across the head of what was opened from the list, it is a swipe back and not a pull.
        const across = e.clientX - start.x;
        if (of.taken() || (start.across && across > 0 && across >= Math.abs(by) * SIDEWAYS)) {
          start = undefined;
          return;
        }
        pulling = true;
        // The page behind shows around the sheet while it moves.
        document.body.classList.add("chat-moving");
        // The peek goes away as the sheet opens, so the sheet itself follows the finger.
        e.currentTarget.setPointerCapture(e.pointerId);
        if (!start.open) of.setOpen(true);
      }
      const passed = e.timeStamp - start.at;
      if (passed > 0) start.speed = (e.clientY - start.last) / passed;
      start.at = e.timeStamp;
      start.last = e.clientY;
      put(start.open ? by : (of.sheet()?.offsetHeight ?? 0) + by);
    },
    onPointerUp: (e: PointerEvent) => drop(e, false),
    onPointerCancel: (e: PointerEvent) => drop(e, true),
    // Taken from what was pressed by the sheet itself, a capture is lost on the way too.
    onLostPointerCapture: (e: PointerEvent) => e.target === e.currentTarget && drop(e, true),
  };
}
