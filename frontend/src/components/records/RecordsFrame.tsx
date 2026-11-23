import { useNavigate } from "@solidjs/router";
import { createEffect, type JSX, on, Show } from "solid-js";
import { IconButton, LinkButton, SplitPane } from "~/components/ui";
import * as icons from "~/lib/icons";
import { withParams } from "~/lib/params";
import type { Rec } from "~/lib/records";

/**
 * A list of records with the one picked beside it. What is picked is in the
 * page's address, so it survives a reload and can be linked to; the list stays
 * where it was, with what narrows it and how far down it is.
 */
export default function RecordsFrame(props: {
  name: string;
  base: string;
  title: string;
  id: string;
  /** The records in the order the list shows them. */
  order: Rec[];
  detail: (id: string) => JSX.Element;
  children: JSX.Element;
}) {
  const navigate = useNavigate();
  const close = () => navigate(props.base, { scroll: false });
  const place = () => props.order.findIndex((r) => r.id === props.id);
  const step = (by: number) => {
    const to = props.order[place() + by];
    if (to) navigate(withParams(props.base, { id: to.id }), { scroll: false, replace: true });
  };

  // One column, as on a phone, scrolls the pane itself; it goes back to where
  // the list was. Side by side the list never moved, and the row picked is
  // brought into view as the list narrows.
  const pane = () => document.getElementById("pane");
  let left = 0;
  createEffect(
    on(
      () => props.id,
      (now, before) => {
        if (now === before) return;
        if (now && !before) left = pane()?.scrollTop ?? 0;
        queueMicrotask(() => {
          pane()?.scrollTo(0, now ? 0 : left);
          document.querySelector("[data-record].current")?.scrollIntoView({ block: "nearest" });
        });
      },
    ),
  );

  return (
    <SplitPane
      name={props.name}
      open={!!props.id}
      label={`${props.title}の詳細`}
      onClose={close}
      bar={
        <>
          <LinkButton small class="split-back" icon={icons.chevronLeft} href={props.base} noScroll>
            {props.title}の一覧
          </LinkButton>
          <Show when={place() >= 0}>
            <span class="sub">
              {place() + 1} / {props.order.length}
            </span>
            <IconButton variant="ghost" icon={icons.arrowUp} label="前の項目" disabled={place() === 0} onClick={() => step(-1)} />
            <IconButton
              variant="ghost"
              icon={icons.arrowDown}
              label="次の項目"
              disabled={place() === props.order.length - 1}
              onClick={() => step(1)}
            />
          </Show>
          <span class="grow" />
          <IconButton class="split-close" variant="ghost" icon={icons.x} label="詳細を閉じる" onClick={close} />
        </>
      }
      detail={
        <Show when={props.id} keyed>
          {(id) => props.detail(id)}
        </Show>
      }
    >
      {props.children}
    </SplitPane>
  );
}
