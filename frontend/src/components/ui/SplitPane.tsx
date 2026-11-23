import { type JSX, Show } from "solid-js";
import { createResizer } from "./resize";

/** Presses of Escape that are for what is being typed, not for the pane. */
const TYPING = "input, textarea, select, [role=dialog], [role=combobox], [role=listbox]";

/**
 * What there is at the left and what is picked of it at the right, each
 * scrolling by itself. With nothing picked the left has all the room. Where
 * there is no room for both, the detail covers the left (`floats`: lies over
 * its right part), and closing it shows the left as it was.
 */
export default function SplitPane(props: {
  /** What the width of the left is kept under in this browser. */
  name: string;
  open: boolean;
  /** What the detail is called. */
  label: string;
  onClose: () => void;
  floats?: boolean;
  bar: JSX.Element;
  detail: JSX.Element;
  children: JSX.Element;
}) {
  let frame: HTMLDivElement | undefined;
  let left: HTMLDivElement | undefined;
  const size = createResizer({
    key: `famulen.split.${props.name}`,
    width: 0,
    least: 300,
    most: () => (frame?.clientWidth ?? 0) - 380,
    from: (e) => e.clientX - (frame?.getBoundingClientRect().left ?? 0),
    toward: "ArrowRight",
    now: () => left?.clientWidth ?? 0,
  });

  /** Up and down move between the rows, and bring along what is open. */
  const onKeyDown = (e: KeyboardEvent) => {
    if (!(e.target instanceof HTMLElement) || !e.target.matches(".item-pick") || e.metaKey || e.ctrlKey || e.altKey) return;
    const rows = [...(left?.querySelectorAll<HTMLElement>("[data-record] .item-pick") ?? [])].filter((r) => r.offsetParent);
    const at = rows.indexOf(e.target);
    const to = { ArrowDown: at + 1, ArrowUp: at - 1, Home: 0, End: rows.length - 1 }[e.key];
    if (at < 0 || to === undefined) return;
    e.preventDefault();
    const row = rows[Math.max(0, Math.min(rows.length - 1, to))];
    row.focus();
    if (props.open && row !== e.target) {
      // The way back leads out of the list, not through every row passed.
      row.setAttribute("replace", "");
      row.click();
      row.removeAttribute("replace");
    }
  };

  const onEscape = (e: KeyboardEvent) => {
    if (e.key !== "Escape" || e.defaultPrevented) return;
    if (e.target instanceof Element && e.target.closest(TYPING)) return;
    const row = left?.querySelector<HTMLElement>("[data-record].current .item-pick");
    props.onClose();
    row?.focus();
  };

  return (
    <div class="split-pane" classList={{ open: props.open, floats: props.floats }} ref={frame} onKeyDown={(e) => props.open && onEscape(e)}>
      <div
        class="split-left"
        ref={left}
        style={props.open && size.width() ? { "--left": `${size.width()}px` } : undefined}
        onKeyDown={onKeyDown}
      >
        {props.children}
      </div>
      <Show when={props.open}>
        <section class="split-detail" aria-label={props.label}>
          <Show when={!props.floats}>
            <div class="resize-edge" aria-valuenow={size.width() || undefined} {...size.separator} />
          </Show>
          <div class="split-bar">{props.bar}</div>
          <div class="split-scroll">{props.detail}</div>
        </section>
      </Show>
    </div>
  );
}
