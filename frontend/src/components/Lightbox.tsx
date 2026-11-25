import { createSignal, onCleanup, Show } from "solid-js";
import { buttonClass, Icon, IconButton } from "~/components/ui";
import * as icons from "~/lib/icons";
import { freeLayout, holdLayout } from "~/lib/viewport";

type Shown = { name: string; url: string };

const [image, setImage] = createSignal<Shown>();
let box: HTMLDialogElement | undefined;
/** The box being written in as the image opened, and where its caret was. */
let writing: { box: HTMLTextAreaElement; from: number; to: number } | undefined;

/** Open an image, pressed at `opener`, over everything else. */
export function openImage(opened: Shown, opener: HTMLElement) {
  const at = document.activeElement;
  writing = at instanceof HTMLTextAreaElement ? { box: at, from: at.selectionStart, to: at.selectionEnd } : undefined;
  // What has the focus as it opens has it again once it closes. Safari leaves
  // a button that is pressed without it.
  if (!writing) opener.focus({ preventScroll: true });
  // The keyboard goes while the image is looked at. The conversation behind
  // does not follow, so that what is read is there when it comes back.
  holdLayout();
  setImage(opened);
  box?.showModal();
}

/** Put back what opening it took. Closing is told of a little later when the browser closed it, as on Escape. */
function closed() {
  if (box?.open || !image()) return;
  // Chromium gives a text box the focus back without its caret.
  const back = writing && document.activeElement === writing.box;
  if (back) writing?.box.setSelectionRange(writing.from, writing.to);
  writing = undefined;
  freeLayout(!!back);
  setImage();
}

export function closeImage() {
  box?.close();
  closed();
}

/**
 * The image open over everything else. It is the browser's own dialog, not
 * Ark's, as that gives the focus back at once: iOS brings the keyboard up
 * again only from within the tap that closed it.
 */
export default function Lightbox() {
  onCleanup(closeImage);
  return (
    <dialog
      ref={box}
      class="lightbox"
      role="dialog"
      aria-modal="true"
      aria-label={image()?.name}
      onClose={closed}
      onClick={(e) => {
        if (!e.target.closest("img, .lightbox-bar")) closeImage();
      }}
    >
      <Show when={image()}>
        {(shown) => (
          <>
            <div class="lightbox-bar">
              <span class="grow">{shown().name}</span>
              <a class={buttonClass({ small: true })} href={shown().url} target="_blank" rel="noreferrer">
                <Icon svg={icons.externalLink} size={14} />
                新しいタブで開く
              </a>
              <IconButton icon={icons.x} label="閉じる" autofocus onClick={closeImage} />
            </div>
            <div class="lightbox-room">
              <img src={shown().url} alt={shown().name} />
            </div>
          </>
        )}
      </Show>
    </dialog>
  );
}
