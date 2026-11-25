import { type JSX, Show } from "solid-js";
import { chevronRight } from "~/lib/icons";
import Icon from "./Icon";

/** Rows in one frame. */
export function List(props: { label?: string; class?: string; children: JSX.Element }) {
  return (
    <div class={`list ${props.class ?? ""}`} role="list" aria-label={props.label}>
      {props.children}
    </div>
  );
}

/**
 * One row: what leads it, what it is called over what is said of it, and what
 * stands at its end. With `href` its title is a link that takes presses
 * anywhere on the row, so it is reached and opened from the keyboard, opened
 * in a tab of its own, and read out as a link; controls in the row stay above
 * it.
 */
export function ListRow(props: {
  href?: string;
  /** Pressed anywhere outside its links and controls. One of its controls does the same from the keyboard. */
  onPick?: () => void;
  /** It is what is open beside the list. */
  current?: boolean;
  icon?: string;
  lead?: JSX.Element;
  title: JSX.Element;
  sub?: JSX.Element;
  end?: JSX.Element;
  /** Show that the row leads to a page of its own. */
  leads?: boolean;
  class?: string;
  classList?: { [name: string]: boolean | undefined };
  record?: string;
  /** What is below the row's line, at its full width. */
  children?: JSX.Element;
}) {
  return (
    <div
      class={`item ${props.class ?? ""}`}
      classList={{ picks: !!props.href || !!props.onPick, current: props.current, ...props.classList }}
      role="listitem"
      data-record={props.record}
      onClick={(e) => {
        // What ends at the end of a selection is no press.
        const selecting = window.getSelection()?.isCollapsed === false;
        if (props.onPick && !selecting && e.target instanceof Element && !e.target.closest("a, button, input, label")) props.onPick();
      }}
    >
      <Show when={props.icon}>{(svg) => <Icon svg={svg()} size={18} />}</Show>
      {props.lead}
      <div class="item-main">
        <Show
          when={props.href}
          fallback={<div class="item-title">{props.title}</div>}
        >
          {(href) => (
            <a class="item-title item-pick" href={href()} aria-current={props.current ? "true" : undefined}>
              {props.title}
            </a>
          )}
        </Show>
        <Show when={"sub" in props}>
          <div class="item-sub">{props.sub}</div>
        </Show>
      </div>
      <Show when={"end" in props}>
        <div class="item-end">{props.end}</div>
      </Show>
      <Show when={props.leads}>
        <Icon svg={chevronRight} size={16} />
      </Show>
      {props.children}
    </div>
  );
}
