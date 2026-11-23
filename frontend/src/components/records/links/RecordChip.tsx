import { A } from "@solidjs/router";
import { Show } from "solid-js";
import { hrefOf, linkOf } from "~/lib/records";
import { useKnownLink } from "./known";
import LinkedText from "./LinkedText";

/** Words the notebook gives a meaning of their own. */
const WORDS: { [text: string]: string } = { me: "あなた", "previous unfinished": "前の回がまだ済んでいない" };

/** A field's link as a chip that opens the record, called what it is called now. */
export default function RecordChip(props: { text: string }) {
  const known = useKnownLink();
  const link = () => linkOf(props.text);
  return (
    <Show when={link()} fallback={<span>{WORDS[props.text] ?? <LinkedText text={props.text} />}</span>}>
      {(l) => {
        const now = () => known(l().kind, l().id);
        const title = () => now()?.title || l().title || l().id;
        return (
          <Show when={!now()?.missing} fallback={<span class="tag missing">{title()}（もうない）</span>}>
            <Show when={hrefOf(l().kind, l().id)} fallback={<span class="tag">{title()}</span>}>
              {(href) => (
                <A class="tag link" href={href()}>
                  {title()}
                </A>
              )}
            </Show>
          </Show>
        );
      }}
    </Show>
  );
}
