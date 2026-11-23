import { NumberInput as Ark } from "@ark-ui/solid/number-input";
import { Switch as ArkSwitch } from "@ark-ui/solid/switch";
import { type ComponentProps, type JSX, Show, splitProps } from "solid-js";
import { createDraft } from "~/lib/draft";
import { search } from "~/lib/icons";
import Icon from "./Icon";

/** Nothing here is a login of a site, so no password manager fills it in. */
const NO_FILL = { autocomplete: "off", "data-1p-ignore": "true", "data-lpignore": "true", "data-bwignore": "true" } as const;

/** A label over a control, with a plain note under it. */
export function Field(props: { label: JSX.Element; hint?: JSX.Element; group?: boolean; class?: string; children: JSX.Element }) {
  const body = () => (
    <>
      <span class="field-label">{props.label}</span>
      {props.children}
      <Show when={"hint" in props}>
        <span class="sub">{props.hint}</span>
      </Show>
    </>
  );
  return (
    <Show when={props.group} fallback={<label class={`field ${props.class ?? ""}`}>{body()}</label>}>
      <div class={`field ${props.class ?? ""}`}>{body()}</div>
    </Show>
  );
}

export function TextField(props: ComponentProps<"input">) {
  const [own, rest] = splitProps(props, ["class"]);
  return <input {...NO_FILL} {...rest} class={`input ${own.class ?? ""}`} />;
}

/** The text goes in as the content too: the server writes `value` as an attribute, which a textarea ignores, and hydration does not set it. */
export function TextArea(props: Omit<ComponentProps<"textarea">, "value" | "children"> & { value?: string }) {
  const [own, rest] = splitProps(props, ["class", "value"]);
  return (
    <textarea {...NO_FILL} {...rest} class={`input ${own.class ?? ""}`} value={own.value ?? ""}>
      {own.value}
    </textarea>
  );
}

/**
 * A choice out of many. The browser's own control: it opens as a wheel on a
 * phone, is never cut off by what scrolls around it, and needs no script.
 */
export function Select(props: ComponentProps<"select">) {
  const [own, rest] = splitProps(props, ["class", "children"]);
  return (
    <select {...rest} class={`input select ${own.class ?? ""}`}>
      {own.children}
    </select>
  );
}

/** Words to narrow a list by. */
export function SearchBox(props: Omit<ComponentProps<"input">, "aria-label"> & { label: string }) {
  const [own, rest] = splitProps(props, ["label", "class"]);
  return (
    <label class={`search ${own.class ?? ""}`}>
      <Icon svg={search} size={16} />
      <input {...NO_FILL} type="search" {...rest} class="input" aria-label={own.label} />
    </label>
  );
}

export function Checkbox(props: Omit<ComponentProps<"input">, "type"> & { children: JSX.Element }) {
  const [own, rest] = splitProps(props, ["children", "class"]);
  return (
    <label class={`check ${own.class ?? ""}`}>
      <input {...rest} type="checkbox" class="check-box" />
      <span>{own.children}</span>
    </label>
  );
}

/** On or off, saved as it is turned. `hidden` keeps the label for those who do not see the row it stands in. */
export function Switch(props: { checked: boolean; onChange: (on: boolean) => void; children: JSX.Element; hidden?: boolean; end?: boolean; class?: string }) {
  return (
    <ArkSwitch.Root class={`switch-row ${props.end ? "end" : ""} ${props.class ?? ""}`} checked={props.checked} onCheckedChange={(d) => props.onChange(d.checked)}>
      <ArkSwitch.Control class="switch">
        <ArkSwitch.Thumb class="switch-thumb" />
      </ArkSwitch.Control>
      <ArkSwitch.Label class={props.hidden ? "sr" : ""}>{props.children}</ArkSwitch.Label>
      <ArkSwitch.HiddenInput />
    </ArkSwitch.Root>
  );
}

/** A number saved when its field is left or Enter is pressed; an empty or broken one goes back to what it was. */
export function NumberField(props: {
  label: string;
  /** The label is there, but not shown. */
  hidden?: boolean;
  value: number;
  min?: number;
  max?: number;
  unit?: JSX.Element;
  hint?: JSX.Element;
  /** Shown for zero, where zero means there is none. */
  none?: string;
  id?: string;
  onSave: (n: number) => void;
}) {
  const shown = () => (props.none !== undefined && props.value === 0 ? "" : String(props.value));
  const [text, setText] = createDraft(shown);
  return (
    <Ark.Root
      class="field"
      ids={props.id ? { input: props.id } : undefined}
      min={props.min}
      max={props.max}
      value={text()}
      onValueChange={(d) => setText(d.value)}
      onValueCommit={(d) => {
        const n = Math.trunc(d.valueAsNumber);
        if (Number.isFinite(n) && n >= (props.min ?? 0)) {
          if (n !== props.value) props.onSave(n);
        } else if (props.none !== undefined) {
          if (props.value !== 0) props.onSave(0);
        } else setText(shown());
      }}
    >
      <Ark.Label class={props.hidden ? "sr" : "field-label"}>{props.label}</Ark.Label>
      <div class="row">
        <Ark.Control>
          <Ark.Input class="input number" inputmode="numeric" placeholder={props.none} />
        </Ark.Control>
        <Show when={"unit" in props}>
          <span class="sub">{props.unit}</span>
        </Show>
      </div>
      <Show when={"hint" in props}>
        <span class="sub">{props.hint}</span>
      </Show>
    </Ark.Root>
  );
}
