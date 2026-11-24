import { Show } from "solid-js";
import { Button, Callout } from "~/components/ui";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { hrefOf, KIND_NAMES, type Rec } from "~/lib/records";

/** That a record may be out of date, or lives on as another, above all it says. */
export default function Standing(props: { record: Rec }) {
  const r = () => props.record;
  return (
    <>
      <Show when={r().obsolete}>
        <Callout
          tone="warn"
          action={
            <Button small onClick={() => void act(() => api.records.clearObsolete({ kind: r().kind, id: r().id }), "印を外した")}>
              確かめたので印を外す
            </Button>
          }
        >
          古くなったかもしれない（{r().obsolete}）{r().obsoleteReason ? `：${r().obsoleteReason}` : ""}
        </Callout>
      </Show>
      <Show when={r().movedTo}>
        {(to) => (
          <Callout>
            {KIND_NAMES[to().kind] ?? to().kind}に移した：
            <a href={hrefOf(to().kind, to().id) ?? "#"}>{to().title}</a>
          </Callout>
        )}
      </Show>
    </>
  );
}
