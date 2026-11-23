import { Dialog } from "@ark-ui/solid/dialog";
import { type JSX, Show } from "solid-js";
import { Portal } from "solid-js/web";

/** A dialog opened by its trigger, holding `children`. */
export default function Modal(props: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description?: string;
  trigger?: JSX.Element;
  /** `wide` lets the dialog grow on a large screen, for picking among many things. */
  size?: "wide";
  children: JSX.Element;
}) {
  return (
    <Dialog.Root open={props.open} onOpenChange={(d) => props.onOpenChange(d.open)} lazyMount unmountOnExit>
      {props.trigger}
      <Portal>
        <Dialog.Backdrop class="dialog-backdrop" />
        <Dialog.Positioner class="dialog-positioner">
          <Dialog.Content class="dialog" classList={{ wide: props.size === "wide" }}>
            <Dialog.Title class="dialog-title">{props.title}</Dialog.Title>
            <Show when={props.description}>
              <Dialog.Description class="sub">{props.description}</Dialog.Description>
            </Show>
            {props.children}
          </Dialog.Content>
        </Dialog.Positioner>
      </Portal>
    </Dialog.Root>
  );
}
