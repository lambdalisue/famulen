import { createSignal, onCleanup, onMount, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { Icon } from "~/components/ui";
import type { CalendarEvent } from "~/gen/famulen/v1/calendar_pb";
import * as icons from "~/lib/icons";
import { excerptOf } from "~/lib/records";
import { whenOf } from "./EventPanel";

const WIDTH = 300;
const EDGE = 8;

/**
 * The card an event grows into under the pointer, or with the focus, and what
 * an event carries to show it. It takes no pointer of its own, so what lies
 * under it is still reached.
 */
export function createPeek() {
  const [at, setAt] = createSignal<{ event: CalendarEvent; box: DOMRect }>();
  const hide = () => setAt(undefined);
  const show = (event: CalendarEvent, el: HTMLElement) => setAt({ event, box: el.getBoundingClientRect() });
  onMount(() => {
    // What it stands on moves away under it.
    window.addEventListener("scroll", hide, { capture: true, passive: true });
    onCleanup(() => window.removeEventListener("scroll", hide, { capture: true }));
  });
  return {
    shown: at,
    hide,
    of: (event: CalendarEvent) => ({
      onPointerEnter: (e: PointerEvent & { currentTarget: HTMLElement }) => {
        if (e.pointerType === "mouse") show(event, e.currentTarget);
      },
      onPointerLeave: hide,
      onFocus: (e: FocusEvent & { currentTarget: HTMLElement }) => {
        if (e.currentTarget.matches(":focus-visible")) show(event, e.currentTarget);
      },
      onBlur: hide,
    }),
  };
}

export type Peek = ReturnType<typeof createPeek>;

function Card(props: { event: CalendarEvent; box: DOMRect }) {
  const e = () => props.event;
  let card: HTMLDivElement | undefined;
  const [top, setTop] = createSignal(0);
  const width = () => Math.max(WIDTH, Math.min(props.box.width, 420));
  const left = () => Math.max(EDGE, Math.min(props.box.left, window.innerWidth - width() - EDGE));
  onMount(() => setTop(Math.max(EDGE, Math.min(props.box.top, window.innerHeight - (card?.offsetHeight ?? 0) - EDGE))));
  return (
    <div
      class="ev-card"
      role="tooltip"
      ref={card}
      style={{ "--c": e().color || undefined, left: `${left()}px`, top: `${top()}px`, width: `${width()}px` }}
    >
      <b>
        {e().status === "hold" ? "仮 · " : ""}
        {e().title || "（題名なし）"}
      </b>
      <span class="ev-card-line">
        <Icon svg={icons.clock} size={14} />
        {whenOf(e())}
      </span>
      <Show when={e().location}>
        <span class="ev-card-line">
          <Icon svg={icons.mapPin} size={14} />
          {e().location}
        </span>
      </Show>
      <Show when={e().attendees.length > 0}>
        <span class="ev-card-line">
          <Icon svg={icons.users} size={14} />
          {e().attendees.slice(0, 4).map((a) => a.name || a.email).join("、")}
          {e().attendees.length > 4 ? ` ほか ${e().attendees.length - 4} 人` : ""}
        </span>
      </Show>
      <Show when={e().calendarName}>
        <span class="ev-card-line">
          <Icon svg={icons.calendar} size={14} />
          {e().calendarName}
        </span>
      </Show>
      <Show when={excerptOf(e().notes, 140)}>{(notes) => <span class="ev-card-notes">{notes()}</span>}</Show>
    </div>
  );
}

export default function EventPeek(props: { peek: Peek }) {
  return (
    <Show when={props.peek.shown()} keyed>
      {(at) => (
        <Portal>
          <Card event={at.event} box={at.box} />
        </Portal>
      )}
    </Show>
  );
}
