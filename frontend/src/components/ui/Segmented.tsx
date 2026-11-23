import { SegmentGroup } from "@ark-ui/solid/segment-group";
import { For } from "solid-js";

/** One choice out of a few, each shown. */
export function Segmented(props: { label: string; items: [string, string][]; value: string; onChoose: (value: string) => void; class?: string }) {
  return (
    <SegmentGroup.Root
      class={`segs ${props.class ?? ""}`}
      aria-label={props.label}
      value={props.value}
      onValueChange={(d) => {
        if (d.value !== null && d.value !== props.value) props.onChoose(d.value);
      }}
    >
      <SegmentGroup.Indicator class="segs-indicator" />
      <For each={props.items}>
        {([value, text]) => (
          <SegmentGroup.Item value={value} class="segs-item">
            <SegmentGroup.ItemText>{text}</SegmentGroup.ItemText>
            <SegmentGroup.ItemControl />
            <SegmentGroup.ItemHiddenInput />
          </SegmentGroup.Item>
        )}
      </For>
    </SegmentGroup.Root>
  );
}

/** {@link Segmented} over values that are not strings, such as an API enum. */
export function SegmentedOf<T>(props: { label: string; items: readonly (readonly [T, string])[]; value: T; onChoose: (value: T) => void }) {
  return (
    <Segmented
      label={props.label}
      items={props.items.map(([v, text]) => [String(v), text])}
      value={String(props.value)}
      onChoose={(key) => {
        const chosen = props.items.find(([v]) => String(v) === key);
        if (chosen) props.onChoose(chosen[0]);
      }}
    />
  );
}

/** Segments that are links: each shows another view of the page. */
export function SegmentedLinks(props: { label: string; items: [href: string, text: string, on: boolean][]; class?: string }) {
  return (
    <nav class={`segs ${props.class ?? ""}`} aria-label={props.label}>
      <For each={props.items}>
        {([href, text, on]) => (
          <a class="segs-item" classList={{ on }} href={href} aria-current={on ? "true" : undefined}>
            {text}
          </a>
        )}
      </For>
    </nav>
  );
}
