import { For } from "solid-js";
import { say } from "~/lib/act";
import * as icons from "~/lib/icons";
import { linkOf, linkTo, type Rec, RELATED, valuesOf } from "~/lib/records";
import RecordPicker from "../RecordPicker";
import RecordChip from "./RecordChip";
import { Icon, Tag } from "~/components/ui";

/** The records of any kind this one has to do with, added and removed. */
export default function RelatedLinks(props: { record: Rec; onSave: (change: (values: string[]) => string[]) => void }) {
  const values = () => valuesOf(props.record, RELATED);
  const add = (r: Rec) => {
    const same = (l: { kind: string; id: string } | undefined) => l?.kind === r.kind && l.id === r.id;
    if (same(props.record)) return say("自分自身にはつなげない");
    if (values().some((v) => same(linkOf(v))) || props.record.backlinks.some((b) => b.from === RELATED && same(b))) {
      return say("もうつながっている");
    }
    props.onSave((now) => [...now, linkTo(r)]);
  };
  return (
    <section class="rec-section">
      <h3>関わるもの</h3>
      <span class="chips-edit">
        <For each={values()}>
          {(v) => (
            <Tag class="removable">
              <RecordChip text={v} />
              <button
                type="button"
                aria-label={`${linkOf(v)?.title || v}を関わるものから外す`}
                onClick={() => props.onSave((now) => now.filter((x) => x !== v))}
              >
                <Icon svg={icons.x} size={12} />
              </button>
            </Tag>
          )}
        </For>
        <RecordPicker label="関わるものを足す" onPick={add} />
      </span>
    </section>
  );
}
