import { type ComponentProps, createEffect, createSignal, type JSX, mergeProps, Show, splitProps } from "solid-js";
import { chevronLeft } from "~/lib/icons";
import Icon from "./Icon";
import { useSending } from "./Sending";

/** `primary` is the one main action where it stands, `danger` removes, `ghost` stands back. */
export type ButtonVariant = "primary" | "danger" | "ghost";

type Look = { variant?: ButtonVariant; small?: boolean; icon?: string };

/** The class of a button, for what only takes a class, such as an Ark trigger. */
export function buttonClass(look: Look = {}, more = ""): string {
  return ["btn", look.small && "small", look.variant, more].filter(Boolean).join(" ");
}

type Sends = {
  /** Stay unpressable once what it sends is done, as what it was pressed on goes away. */
  hold?: boolean;
};

type Pressed = {
  onClick?: JSX.EventHandlerUnion<HTMLButtonElement, MouseEvent>;
  disabled?: boolean;
  hold?: boolean;
  "aria-busy"?: boolean | "true" | "false";
  type?: ComponentProps<"button">["type"];
};

/**
 * What a press does: when it gives back a promise, the button and those it is
 * sent together with cannot be pressed until it settles, and the one pressed
 * says it is busy. A form's submit button is busy while the form sends.
 */
export function createPress(props: Pressed) {
  const submits = props.type === "submit";
  const group = useSending(submits);
  const [mine, setMine] = createSignal(false);
  createEffect(() => {
    if (!group.sending.busy()) setMine(false);
  });
  const sendsForm = submits && group.form === true;
  const onClick = (e: MouseEvent & { currentTarget: HTMLButtonElement; target: Element }) => {
    const h = props.onClick;
    if (!h) return;
    const press = (): unknown => (typeof h === "function" ? h(e) : h[0](h[1], e));
    // What the form sends follows the click: were the click to take the
    // form's turn, the submit after it would be refused.
    if (sendsForm) {
      press();
      return;
    }
    const sent = group.sending.run(press, props.hold ?? group.hold);
    if (sent !== undefined && group.sending.busy()) setMine(true);
  };
  return {
    onClick,
    disabled: () => props.disabled || group.sending.busy(),
    busy: () => mine() || (sendsForm && group.sending.busy()) || props["aria-busy"] || undefined,
  };
}

/**
 * A button. It submits nothing unless told to with `type="submit"`. A press
 * that sends, by giving back a promise, is not done again until it is back.
 */
export function Button(props: ComponentProps<"button"> & Look & Sends) {
  const [look, own, sends, rest] = splitProps(props, ["variant", "small", "icon"], ["class", "children", "type"], ["onClick", "disabled", "hold", "aria-busy"]);
  const press = createPress(
    mergeProps(sends, {
      get type() {
        return own.type;
      },
    }),
  );
  return (
    <button
      {...rest}
      type={own.type ?? "button"}
      class={buttonClass(look, own.class)}
      disabled={press.disabled()}
      aria-busy={press.busy()}
      onClick={press.onClick}
    >
      <Show when={look.icon}>{(svg) => <Icon svg={svg()} size={look.small ? 14 : 16} />}</Show>
      {own.children}
    </button>
  );
}

/** A button of an icon alone; its label is what it is called and what the pointer shows. */
export function IconButton(
  props: Omit<ComponentProps<"button">, "aria-label" | "title"> &
    Sends & {
      icon: string;
      label: string;
      title?: string;
      variant?: ButtonVariant;
      plain?: boolean;
      size?: number;
    },
) {
  const [own, sends, rest] = splitProps(props, ["icon", "label", "title", "variant", "plain", "size", "class", "type"], ["onClick", "disabled", "hold", "aria-busy"]);
  const press = createPress(
    mergeProps(sends, {
      get type() {
        return own.type;
      },
    }),
  );
  return (
    <button
      {...rest}
      type={own.type ?? "button"}
      disabled={press.disabled()}
      aria-busy={press.busy()}
      onClick={press.onClick}
      class={
        own.plain
          ? `icon-plain ${own.variant ?? ""} ${own.class ?? ""}`
          : buttonClass({ small: true, variant: own.variant }, `icon-btn ${own.class ?? ""}`)
      }
      aria-label={own.label}
      title={own.title ?? own.label}
    >
      <Icon svg={own.icon} size={own.size ?? 16} />
    </button>
  );
}

/** A link that looks like a button: it leads somewhere instead of doing something. */
export function LinkButton(props: ComponentProps<"a"> & Look & { href: string; label?: string; children?: JSX.Element }) {
  const [look, own, rest] = splitProps(props, ["variant", "small", "icon"], ["class", "children", "label"]);
  const outside = () => /^https?:/.test(rest.href);
  return (
    <a
      {...rest}
      class={buttonClass(look, `${own.label && !own.children ? "icon-btn " : ""}${own.class ?? ""}`)}
      aria-label={own.label}
      title={own.label}
      target={outside() ? "_blank" : undefined}
      rel={outside() ? "noreferrer" : undefined}
    >
      <Show when={look.icon}>{(svg) => <Icon svg={svg()} size={look.small ? 14 : 16} />}</Show>
      {own.children}
    </a>
  );
}

/** The way back from a page to the list it is in. */
export function BackLink(props: { href: string; label: string }) {
  return (
    <a class={buttonClass({ small: true }, "icon-btn back-link")} href={props.href} aria-label={props.label} title={props.label}>
      <Icon svg={chevronLeft} size={16} />
    </a>
  );
}
