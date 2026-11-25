import { Show } from "solid-js";
import { memoryHref } from "~/lib/memory";
import { hrefOf, KIND_NAMES, nameOf, plainText, type Rec, RELATED } from "~/lib/records";
import { Keyed, List, ListRow } from "~/components/ui";

/** What a link to the record from a field says the other is to it. */
const FROM: { [from: string]: string } = {
  Parent: "子のタスク",
  "Blocked by": "これを待つタスク",
  "Member of": "属する人",
  Relations: "関係",
  Covers: "これを対象にする契約",
  Attendees: "参加する予定",
  Assignee: "任せているタスク",
  Calendar: "予定",
};

/** Places any kind has, so the kind is told too. */
const ANYWHERE: { [from: string]: string } = {
  [RELATED]: "関わる",
  Notes: "備考で触れている",
  Details: "ほかの項目で触れている",
  Log: "経過で触れている",
  "": "触れている",
};

/** Everything that links to a record, by where and from which kind. */
export default function Backlinks(props: { record: Rec }) {
  const groups = () => {
    const by = new Map<string, Rec["backlinks"]>();
    for (const l of props.record.backlinks) {
      const kind = KIND_NAMES[l.kind] ?? l.kind;
      const key = l.from in ANYWHERE ? `${ANYWHERE[l.from]}${kind}` : (FROM[l.from] ?? `${nameOf(l.from)}（${kind}）`);
      by.set(key, [...(by.get(key) ?? []), l]);
    }
    return [...by];
  };
  // A party's page shows its long-term memories by themselves.
  const memories = () => props.record.memories.filter((m) => props.record.kind !== "parties" || !m.file.startsWith("long/"));
  return (
    <Show when={groups().length > 0 || memories().length > 0}>
      <section class="rec-section">
        <h3>つながり</h3>
        <Keyed each={groups()} by={([name]) => name}>
          {(group) => (
            <div class="rec-links">
              <span class="sub">{group()[0]}</span>
              <List label={group()[0]}>
                <Keyed each={group()[1]} by={(l) => `${l.kind}/${l.id}`}>
                  {(l) => <ListRow href={hrefOf(l().kind, l().id)} title={l().title} />}
                </Keyed>
              </List>
            </div>
          )}
        </Keyed>
        <Show when={memories().length > 0}>
          <div class="rec-links">
            <span class="sub">触れている記憶</span>
            <List label="触れている記憶">
              <Keyed each={memories()} by={(m) => `${m.file}#${m.id}`}>
                {(m) => <ListRow href={memoryHref(`memory/${m().file}`) ?? "/memory"} title={plainText(m().title)} />}
              </Keyed>
            </List>
          </div>
        </Show>
      </section>
    </Show>
  );
}
