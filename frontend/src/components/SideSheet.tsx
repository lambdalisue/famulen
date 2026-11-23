import { Dialog } from "@ark-ui/solid/dialog";
import { createSignal, type JSX, onMount } from "solid-js";
import { Portal } from "solid-js/web";
import Icon from "~/components/Icon";
import * as icons from "~/lib/icons";

const NARROWEST = 320;
/** How wide every sheet opens until the owner drags it, and what the reset returns to. */
const WIDTH = 720;
const STEP = 24;

/** Room left of the sheet so the page behind it can still be pressed to close it. */
const widest = () => Math.max(NARROWEST, window.innerWidth - 48);

const clamp = (width: number) => Math.round(Math.min(widest(), Math.max(NARROWEST, width)));

/**
 * The sheet from the right that an Ark `Dialog.Root` opens: a header with the
 * title, and an edge to drag to change its width. The width is kept in the
 * browser under `name`, so the sheet opens as wide as it was last left.
 */
export default function SideSheet(props: {
  name: string;
  title: string;
  children: JSX.Element;
}) {
  const key = () => `famulen.sheet.${props.name}`;
  const [width, setWidth] = createSignal(WIDTH);
  onMount(() => {
    const kept = Number(localStorage.getItem(key()));
    if (kept > 0) setWidth(clamp(kept));
  });
  const keep = (w: number) => {
    setWidth(w);
    if (w === WIDTH) localStorage.removeItem(key());
    else localStorage.setItem(key(), String(w));
  };

  let dragging = false;
  const edge = {
    onPointerDown: (e: PointerEvent & { currentTarget: HTMLElement }) => {
      e.preventDefault();
      e.currentTarget.setPointerCapture(e.pointerId);
      dragging = true;
    },
    onPointerMove: (e: PointerEvent) => {
      if (dragging) setWidth(clamp(window.innerWidth - e.clientX));
    },
    onPointerUp: (e: PointerEvent & { currentTarget: HTMLElement }) => {
      if (!dragging) return;
      dragging = false;
      e.currentTarget.releasePointerCapture(e.pointerId);
      keep(width());
    },
    onKeyDown: (e: KeyboardEvent) => {
      const by = { ArrowLeft: STEP, ArrowRight: -STEP }[e.key];
      if (by === undefined) return;
      e.preventDefault();
      keep(clamp(width() + by));
    },
  };

  return (
    <Portal>
      <Dialog.Backdrop class="status-backdrop" />
      <Dialog.Positioner class="status-positioner">
        <Dialog.Content class="status-pop" style={{ width: `min(${width()}px, 100vw)` }}>
          <div
            class="side-sheet-edge"
            role="separator"
            aria-orientation="vertical"
            aria-label="幅を変える"
            aria-valuemin={NARROWEST}
            aria-valuenow={width()}
            tabindex="0"
            onPointerDown={edge.onPointerDown}
            onPointerMove={edge.onPointerMove}
            onPointerUp={edge.onPointerUp}
            onPointerCancel={edge.onPointerUp}
            onDblClick={() => keep(WIDTH)}
            onKeyDown={edge.onKeyDown}
          />
          <header class="status-pop-head">
            <Dialog.Title class="status-pop-title">{props.title}</Dialog.Title>
            <div class="side-sheet-tools">
              <button
                type="button"
                class="btn small ghost side-sheet-reset"
                aria-label="元の幅に戻す"
                title="元の幅に戻す"
                disabled={width() === WIDTH}
                onClick={() => keep(WIDTH)}
              >
                <Icon svg={icons.arrowAutofitWidth} size={16} />
              </button>
              <Dialog.CloseTrigger class="btn small ghost" aria-label="閉じる">
                <Icon svg={icons.x} size={16} />
              </Dialog.CloseTrigger>
            </div>
          </header>
          <div class="side-sheet-body">{props.children}</div>
        </Dialog.Content>
      </Dialog.Positioner>
    </Portal>
  );
}
