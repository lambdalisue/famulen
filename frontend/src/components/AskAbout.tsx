import * as icons from "~/lib/icons";
import { setSelection } from "~/lib/view";
import { IconButton } from "~/components/ui";

/** Puts something on screen beside what the owner says next. */
export default function AskAbout(props: { what: string; label?: string; framed?: boolean }) {
  const label = () => props.label ?? "会話に添える";
  return (
    <IconButton
      plain={!props.framed}
      icon={icons.messagePlus}
      label={`${props.what} ― ${label()}`}
      title={label()}
      onClick={() => setSelection(props.what)}
    />
  );
}
