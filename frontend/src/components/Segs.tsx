import { SegmentGroup } from "@ark-ui/solid/segment-group";
import { For } from "solid-js";

/** One choice out of a few, each shown. */
export default function Segs(props: { label: string; items: [string, string][]; value: string; onChoose: (value: string) => void }) {
  return (
    <SegmentGroup.Root
      class="segs"
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
