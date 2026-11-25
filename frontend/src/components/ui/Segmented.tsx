import { SegmentGroup } from "@ark-ui/solid/segment-group";
import { For, Show } from "solid-js";
import Icon from "./Icon";
import Keyed from "./Keyed";

/** One choice out of a few, each shown. */
export function Segmented(props: {
  label: string;
  items: [value: string, text: string, icon?: string][];
  /** Show the icons alone; the words are still read out. */
  brief?: boolean;
  value: string;
  onChoose: (value: string) => void;
  class?: string;
}) {
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
        {([value, text, icon]) => (
          <SegmentGroup.Item value={value} class="segs-item" title={props.brief ? text : undefined}>
            <Show when={icon}>{(svg) => <Icon svg={svg()} size={16} />}</Show>
            <SegmentGroup.ItemText class={props.brief ? "sr" : undefined}>{text}</SegmentGroup.ItemText>
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
      <Keyed each={props.items} by={([href]) => href}>
        {(item) => (
          <a class="segs-item" classList={{ on: item()[2] }} href={item()[0]} aria-current={item()[2] ? "true" : undefined}>
            {item()[1]}
          </a>
        )}
      </Keyed>
    </nav>
  );
}
