import { createEffect, For } from "solid-js";
import { useOwner } from "~/lib/owner";
import { iconFile, type Portrait } from "~/lib/portrait";

const LINKS = [
  { file: "manifest.webmanifest", rel: "manifest" },
  { file: "apple-touch-icon.png", rel: "apple-touch-icon" },
  { file: "favicon.ico", rel: "icon", sizes: "16x16" },
  { file: "favicon-16.png", rel: "icon", type: "image/png", sizes: "16x16" },
  { file: "favicon-32.png", rel: "icon", type: "image/png", sizes: "32x32" },
  { file: "app-icon.svg", rel: "icon", type: "image/svg+xml", sizes: "any" },
];

/** The icons of the tab and the home screen, in the head the server sends. */
export default function PageIcons(props: { portrait: Portrait }) {
  return (
    <For each={LINKS}>
      {(l) => <link data-icon={l.file} rel={l.rel} type={l.type} sizes={l.sizes} href={iconFile(props.portrait, l.file)} />}
    </For>
  );
}

/** Keep those icons to the portrait in the browser, which the document is not rendered in again. */
export function followPortrait() {
  const owner = useOwner();
  createEffect(() => {
    if (!owner.persona()) return;
    const portrait = owner.portrait();
    for (const link of document.head.querySelectorAll<HTMLLinkElement>("link[data-icon]")) {
      const href = iconFile(portrait, link.dataset.icon ?? "");
      // Only a change: setting the same address again makes some browsers fetch it again.
      if (link.getAttribute("href") !== href) link.setAttribute("href", href);
    }
  });
}
