import { Dialog } from "@ark-ui/solid/dialog";
import type { JSX } from "solid-js";
import * as icons from "~/lib/icons";
import { DialogTrigger, SideSheet } from "~/components/ui";

/**
 * The 「ログを見る」 button at the foot of an agent's page, and the sheet from
 * the right it opens. The log is read only while the sheet is open.
 */
export default function LogSheet(props: { title: string; label?: string; children: JSX.Element }) {
  return (
    <Dialog.Root lazyMount unmountOnExit>
      <div class="log-open">
        <DialogTrigger icon={icons.listDetails}>
          {props.label ?? "ログを見る"}
        </DialogTrigger>
      </div>
      <SideSheet name="log" title={props.title}>
        {props.children}
      </SideSheet>
    </Dialog.Root>
  );
}
