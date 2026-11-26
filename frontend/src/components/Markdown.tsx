import { useLocation, useNavigate } from "@solidjs/router";
import { createEffect, createSignal, onMount, Show } from "solid-js";
import { Dynamic } from "solid-js/web";
import type { RecordLink } from "~/gen/famulen/v1/records_pb";
import { draw, notebookHref } from "~/lib/markdown";
import { addressOf } from "~/lib/params";
import { isNarrow } from "~/lib/viewport";

/**
 * Markdown drawn as Arto draws it; until that is ready, the text itself.
 * `inline` sets it into the line it stands in, such as a title or an item of
 * a list; `attributes` sets apart what memory items carry at their end; `links`
 * says what the records it links to are called now; `plain` is what stands
 * for `text` until it is drawn.
 */
export default function Markdown(props: { text: string; plain?: string; inline?: boolean; attributes?: boolean; links?: RecordLink[] }) {
  let viewer: HTMLElement | undefined;
  const navigate = useNavigate();
  const location = useLocation();
  const [drawn, setDrawn] = createSignal(false);
  onMount(() => {
    window.handleMarkdownLinkClick = (path) => {
      const href = notebookHref(path);
      // Where the page already is, the router would add an entry of the same address, which the chat over the page on a
      // phone is closed for instead.
      if (!href) return;
      if (!isNarrow() || addressOf(href) !== `${location.pathname}${location.search}${location.hash}`) navigate(href);
    };
    let turn = 0;
    createEffect(() => {
      const text = props.text;
      const how = { inline: props.inline, attributes: props.attributes, links: props.links };
      const mine = ++turn;
      const place = document.createElement("div");
      if (!viewer) return;
      const target = viewer;
      draw(place, text, how)
        .then(() => {
          if (mine !== turn) return;
          target.replaceChildren(...place.childNodes);
          window.ArtoRenderer?.render(target);
          setDrawn(true);
        })
        .catch(() => setDrawn(false));
    });
  });
  // Following a link does not also pick the item the text stands in.
  const onClick = (e: MouseEvent) => {
    if (e.target instanceof Element && e.target.closest(".md-link, a")) e.stopPropagation();
  };
  const onKeyDown = (e: KeyboardEvent) => {
    const path = e.key === "Enter" && e.target instanceof HTMLElement ? e.target.dataset.mdLink : undefined;
    if (!path) return;
    e.stopPropagation();
    window.handleMarkdownLinkClick?.(path, 0);
  };
  return (
    <Dynamic component={props.inline ? "span" : "div"} class={props.inline ? "md md-inline" : "md"} onClick={onClick} onKeyDown={onKeyDown}>
      <Show when={!drawn()}>
        <Dynamic component={props.inline ? "span" : "pre"} class="md-plain">
          {props.plain ?? props.text}
        </Dynamic>
      </Show>
      <Dynamic
        component={props.inline ? "span" : "div"}
        class="markdown-viewer"
        data-theme="light"
        ref={(el: HTMLElement) => {
          viewer = el;
        }}
        hidden={!drawn()}
      />
    </Dynamic>
  );
}
