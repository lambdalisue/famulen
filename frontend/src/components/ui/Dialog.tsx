import { Dialog as Ark } from "@ark-ui/solid/dialog";
import { type JSX, Show } from "solid-js";
import { Portal } from "solid-js/web";
import * as icons from "~/lib/icons";
import { Button, type ButtonVariant, buttonClass } from "./Button";
import Icon from "./Icon";

/** A dialog over the page, opened by its trigger or by `open`. */
export function Dialog(props: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description?: JSX.Element;
  trigger?: JSX.Element;
  /** `wide` lets the dialog grow on a large screen, for picking among many things. */
  size?: "wide";
  /** What has the focus once it closes; what opened it otherwise. */
  focusAfter?: () => HTMLElement | null;
  alert?: boolean;
  children?: JSX.Element;
}) {
  return (
    <Ark.Root
      open={props.open}
      onOpenChange={(d) => props.onOpenChange(d.open)}
      finalFocusEl={props.focusAfter}
      role={props.alert ? "alertdialog" : "dialog"}
      lazyMount
      unmountOnExit
    >
      {props.trigger}
      <Portal>
        <Ark.Backdrop class="dialog-backdrop" />
        <Ark.Positioner class="dialog-positioner">
          <Ark.Content class="dialog" classList={{ wide: props.size === "wide" }}>
            <header class="dialog-head">
              <Ark.Title class="dialog-title">{props.title}</Ark.Title>
              <Show when={!props.alert}>
                <Ark.CloseTrigger class={buttonClass({ small: true, variant: "ghost" }, "icon-btn")} aria-label="閉じる" title="閉じる">
                  <Icon svg={icons.x} size={16} />
                </Ark.CloseTrigger>
              </Show>
            </header>
            <Show when={props.description}>
              <Ark.Description class="sub">{props.description}</Ark.Description>
            </Show>
            {props.children}
          </Ark.Content>
        </Ark.Positioner>
      </Portal>
    </Ark.Root>
  );
}

/** The button that opens the dialog it is the trigger of. */
export function DialogTrigger(props: { variant?: ButtonVariant; small?: boolean; icon?: string; label?: string; children?: JSX.Element }) {
  return (
    <Ark.Trigger class={buttonClass(props, props.label ? "icon-btn" : "")} aria-label={props.label} title={props.label}>
      <Show when={props.icon}>{(svg) => <Icon svg={svg()} size={props.small ? 14 : 16} />}</Show>
      {props.children}
    </Ark.Trigger>
  );
}

/** The foot of a dialog's form: give up at its left, go on at its right. */
export function DialogActions(props: { submit?: string; disabled?: boolean; cancel?: string; before?: JSX.Element; variant?: ButtonVariant; onSubmit?: () => void }) {
  return (
    <div class="dialog-actions">
      {props.before ?? <Ark.CloseTrigger class={buttonClass()}>{props.cancel ?? "やめる"}</Ark.CloseTrigger>}
      <Show when={props.submit}>
        {(text) => (
          <Button type={props.onSubmit ? "button" : "submit"} variant={props.variant ?? "primary"} disabled={props.disabled} onClick={() => props.onSubmit?.()}>
            {text()}
          </Button>
        )}
      </Show>
    </div>
  );
}
