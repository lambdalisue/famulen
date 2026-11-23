import { useNavigate } from "@solidjs/router";
import { createEffect, createSignal, onMount, Show } from "solid-js";
import { draw, recordHref } from "~/lib/markdown";

/** Markdown drawn as Arto draws it; until that is ready, the text itself. */
export default function Markdown(props: { text: string }) {
  let viewer: HTMLDivElement | undefined;
  const navigate = useNavigate();
  const [drawn, setDrawn] = createSignal(false);
  onMount(() => {
    window.handleMarkdownLinkClick = (path) => {
      const href = recordHref(path);
      if (href) navigate(href);
    };
    let turn = 0;
    createEffect(() => {
      const text = props.text;
      const mine = ++turn;
      const place = document.createElement("div");
      if (!viewer) return;
      const target = viewer;
      draw(place, text)
        .then(() => {
          if (mine !== turn) return;
          target.replaceChildren(...place.childNodes);
          window.ArtoRenderer?.render(target);
          setDrawn(true);
        })
        .catch(() => setDrawn(false));
    });
  });
  return (
    <div class="md">
      <Show when={!drawn()}>
        <pre class="md-plain">{props.text}</pre>
      </Show>
      <div class="markdown-viewer" data-theme="light" ref={viewer} hidden={!drawn()} />
    </div>
  );
}
