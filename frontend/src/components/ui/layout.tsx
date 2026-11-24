import { Collapsible } from "@ark-ui/solid/collapsible";
import { For, type JSX, Show } from "solid-js";
import { chevronLeft, chevronRight } from "~/lib/icons";
import { BackLink, Button } from "./Button";
import Icon from "./Icon";

/** The head of a page: its name, what it is for, and what is done on it. */
export function PageHeader(props: {
  title: JSX.Element;
  note?: JSX.Element;
  back?: { href: string; label: string };
  tags?: JSX.Element;
  actions?: JSX.Element;
}) {
  return (
    <header class="page-head">
      <div class="page-head-line">
        <Show when={props.back}>{(b) => <BackLink {...b()} />}</Show>
        <h1>{props.title}</h1>
        {props.tags}
        <Show when={"actions" in props}>
          <div class="page-head-actions">{props.actions}</div>
        </Show>
      </div>
      <Show when={"note" in props}>
        <div class="sub page-note">{props.note}</div>
      </Show>
    </header>
  );
}

/** One thing decided on a page: a heading, a plain note under it, then its controls. */
export function Section(props: {
  title: JSX.Element;
  note?: JSX.Element;
  action?: JSX.Element;
  label?: string;
  level?: 2 | 3;
  id?: string;
  children?: JSX.Element;
}) {
  return (
    <section class="pref" aria-label={props.label}>
      <div class="pref-head">
        <div class="pref-title">
          <Show when={props.level === 3} fallback={<h2 id={props.id}>{props.title}</h2>}>
            <h3 id={props.id}>{props.title}</h3>
          </Show>
          {props.action}
        </div>
        <Show when={"note" in props}>
          <p class="sub">{props.note}</p>
        </Show>
      </div>
      {props.children}
    </section>
  );
}

/** A part of a section, or a group of lists on a page, under a smaller heading. */
export function SubSection(props: {
  title: JSX.Element;
  note?: JSX.Element;
  action?: JSX.Element;
  class?: string;
  children?: JSX.Element;
}) {
  return (
    <div class={`pref-sub ${props.class ?? ""}`}>
      <div class="pref-title">
        <h3>{props.title}</h3>
        {props.action}
      </div>
      <Show when={"note" in props}>
        <p class="sub">{props.note}</p>
      </Show>
      {props.children}
    </div>
  );
}

/** What is out of the way until asked for. */
export function Disclosure(props: {
  title: JSX.Element;
  class?: string;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  startsOpen?: boolean;
  children: JSX.Element;
}) {
  return (
    <Collapsible.Root
      class={`disclosure ${props.class ?? ""}`}
      open={props.open}
      defaultOpen={props.startsOpen}
      onOpenChange={(d) => props.onOpenChange?.(d.open)}
    >
      <Collapsible.Trigger class="disclosure-trigger">
        <Icon svg={chevronRight} size={14} />
        {props.title}
      </Collapsible.Trigger>
      <Collapsible.Content class="disclosure-content">{props.children}</Collapsible.Content>
    </Collapsible.Root>
  );
}

/** The buttons that turn to the newer and the older page. */
export function Pager(props: { number: number; more: boolean; newer: () => void; older: () => void }) {
  return (
    <Show when={props.number > 1 || props.more}>
      <nav class="pager" aria-label="頁送り">
        <Button small icon={chevronLeft} disabled={props.number === 1} onClick={() => props.newer()}>
          新しいほうへ
        </Button>
        <span class="sub">{props.number} 頁目</span>
        <Button small disabled={!props.more} onClick={() => props.older()}>
          古いほうへ
          <Icon svg={chevronRight} size={14} />
        </Button>
      </nav>
    </Show>
  );
}

/** What the colors of a picture mean. */
export function Legend(props: { items: { text: string; style: JSX.CSSProperties }[]; class?: string }) {
  return (
    <ul class={`legend ${props.class ?? ""}`}>
      <For each={props.items}>
        {(i) => (
          <li>
            <i style={i.style} aria-hidden="true" />
            {i.text}
          </li>
        )}
      </For>
    </ul>
  );
}
