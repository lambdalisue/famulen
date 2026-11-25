import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import type { LinkPreview } from "~/gen/famulen/v1/console_pb";
import { linkImageUrl, previewsRead, readPreviews } from "~/lib/links";

function LinkCard(props: { link: LinkPreview }) {
  const [missing, setMissing] = createSignal(false);
  return (
    <a class="link-card" href={props.link.url} target="_blank" rel="noreferrer">
      <span class="link-card-words">
        <span class="link-card-site">{props.link.site}</span>
        <b class="link-card-title">{props.link.title}</b>
        <Show when={props.link.description}>
          <span class="link-card-about">{props.link.description}</span>
        </Show>
      </span>
      <Show when={props.link.image && !missing()}>
        <img src={linkImageUrl(props.link.image)} alt="" loading="lazy" onError={() => setMissing(true)} />
      </Show>
    </a>
  );
}

/** What the links of a message lead to, read once the message is near what is seen. */
export default function LinkCards(props: { message: bigint }) {
  const [links, setLinks] = createSignal<LinkPreview[]>(previewsRead(props.message) ?? []);
  let place: HTMLDivElement | undefined;
  onMount(() => {
    const said = place?.parentElement;
    if (!said || previewsRead(props.message)) return;
    let asking = false;
    const ask = async () => {
      if (asking) return;
      asking = true;
      try {
        const found = await readPreviews(props.message);
        setLinks(found);
        if (found.length > 0) seen.disconnect();
      } catch {
        // The link stays as it is, and is asked about when it is come back to.
      } finally {
        asking = false;
      }
    };
    const seen = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) void ask();
      },
      { root: said.closest(".messages, .thread-said"), rootMargin: "100% 0px" },
    );
    seen.observe(said);
    onCleanup(() => seen.disconnect());
  });
  return (
    <div class="link-cards" ref={place}>
      <For each={links()}>{(link) => <LinkCard link={link} />}</For>
    </div>
  );
}
