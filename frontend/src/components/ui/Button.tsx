import { A } from "@solidjs/router";
import { type ComponentProps, type JSX, Show, splitProps } from "solid-js";
import { chevronLeft } from "~/lib/icons";
import Icon from "./Icon";

/** `primary` is the page's one main action, `go` agrees to something, `danger` removes, `ghost` stands back. */
export type ButtonVariant = "primary" | "go" | "danger" | "ghost";

type Look = { variant?: ButtonVariant; small?: boolean; icon?: string };

/** The class of a button, for what only takes a class, such as an Ark trigger. */
export function buttonClass(look: Look = {}, more = ""): string {
  return ["btn", look.small && "small", look.variant, more].filter(Boolean).join(" ");
}

/** A button. It submits nothing unless told to with `type="submit"`. */
export function Button(props: ComponentProps<"button"> & Look) {
  const [look, own, rest] = splitProps(props, ["variant", "small", "icon"], ["class", "children", "type"]);
  return (
    <button {...rest} type={own.type ?? "button"} class={buttonClass(look, own.class)}>
      <Show when={look.icon}>{(svg) => <Icon svg={svg()} size={look.small ? 14 : 16} />}</Show>
      {own.children}
    </button>
  );
}

/** A button of an icon alone; its label is what it is called and what the pointer shows. */
export function IconButton(props: Omit<ComponentProps<"button">, "aria-label" | "title"> & { icon: string; label: string; title?: string; variant?: ButtonVariant; plain?: boolean; size?: number }) {
  const [own, rest] = splitProps(props, ["icon", "label", "title", "variant", "plain", "size", "class", "type"]);
  return (
    <button
      {...rest}
      type={own.type ?? "button"}
      class={own.plain ? `icon-plain ${own.variant ?? ""} ${own.class ?? ""}` : buttonClass({ small: true, variant: own.variant }, `icon-btn ${own.class ?? ""}`)}
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
    <A class={buttonClass({ small: true }, "icon-btn")} href={props.href} aria-label={props.label} title={props.label}>
      <Icon svg={chevronLeft} size={16} />
    </A>
  );
}
