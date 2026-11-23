import { Dialog } from "@ark-ui/solid/dialog";
import type { JSX } from "solid-js";
import { Portal } from "solid-js/web";
import * as icons from "~/lib/icons";
import { buttonClass, IconButton } from "./Button";
import Icon from "./Icon";
import { createResizer } from "./resize";

/**
 * The sheet from the right that an Ark `Dialog.Root` opens: a header with the
 * title, and an edge to drag to change its width. The width is kept in the
 * browser under `name`, so the sheet opens as wide as it was last left. Room
 * is left beside it so the page behind can still be pressed to close it.
 */
export default function SideSheet(props: { name: string; title: string; children: JSX.Element }) {
  const size = createResizer({
    key: `famulen.sheet.${props.name}`,
    width: 720,
    least: 320,
    most: () => window.innerWidth - 48,
    from: (e) => window.innerWidth - e.clientX,
    toward: "ArrowLeft",
  });
  return (
    <Portal>
      <Dialog.Backdrop class="sheet-backdrop" />
      <Dialog.Positioner class="sheet-positioner">
        <Dialog.Content class="side-sheet" style={{ width: `min(${size.width()}px, 100vw)` }}>
          <div class="resize-edge side-sheet-edge" aria-valuenow={size.width()} {...size.separator} />
          <header class="side-sheet-head">
            <Dialog.Title class="dialog-title">{props.title}</Dialog.Title>
            <div class="side-sheet-tools">
              <IconButton variant="ghost" class="side-sheet-reset" icon={icons.arrowAutofitWidth} label="元の幅に戻す" disabled={!size.changed()} onClick={() => size.reset()} />
              <Dialog.CloseTrigger class={buttonClass({ small: true, variant: "ghost" }, "icon-btn")} aria-label="閉じる" title="閉じる">
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
