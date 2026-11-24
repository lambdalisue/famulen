import { RadioGroup } from "@ark-ui/solid/radio-group";
import { For, type JSX, Show } from "solid-js";
import Icon from "./Icon";

/** One choice out of a few that each need a line to say what they are. */
export function RadioCards(props: {
  label: string;
  items: { id: string; label: string; note: JSX.Element; icon?: string; picture?: string }[];
  value: string;
  onChoose: (id: string) => void;
}) {
  return (
    <RadioGroup.Root class="radio-cards" value={props.value} onValueChange={(d) => props.onChoose(d.value ?? "")}>
      <RadioGroup.Label class="sr">{props.label}</RadioGroup.Label>
      <For each={props.items}>
        {(i) => (
          <RadioGroup.Item value={i.id} class="radio-card">
            <RadioGroup.ItemControl class="radio-dot" />
            <Show when={i.icon}>{(svg) => <Icon svg={svg()} size={18} />}</Show>
            <Show when={i.picture}>{(src) => <img class="radio-picture" src={src()} alt="" width={72} height={72} />}</Show>
            <RadioGroup.ItemText class="radio-text">
              <b>{i.label}</b>
              <span class="sub">{i.note}</span>
            </RadioGroup.ItemText>
            <RadioGroup.ItemHiddenInput />
          </RadioGroup.Item>
        )}
      </For>
    </RadioGroup.Root>
  );
}
