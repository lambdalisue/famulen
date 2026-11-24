import { A } from "@solidjs/router";
import { For, Show } from "solid-js";
import { memoryHref } from "~/lib/memory";
import { hrefOf, KIND_NAMES, nameOf, plainText, type Rec, RELATED } from "~/lib/records";

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
        <For each={groups()}>
          {([name, links]) => (
            <div class="rec-links">
              <span class="sub">{name}</span>
              <span class="chips-line">
                <For each={links}>
                  {(l) => (
                    <Show when={hrefOf(l.kind, l.id)} fallback={<span class="tag">{l.title}</span>}>
                      {(href) => (
                        <A class="tag link" href={href()}>
                          {l.title}
                        </A>
                      )}
                    </Show>
                  )}
                </For>
              </span>
            </div>
          )}
        </For>
        <Show when={memories().length > 0}>
          <div class="rec-links">
            <span class="sub">触れている記憶</span>
            <span class="chips-line">
              <For each={memories()}>
                {(m) => (
                  <A class="tag link" href={memoryHref(`memory/${m.file}`) ?? "/memory"}>
                    {plainText(m.title)}
                  </A>
                )}
              </For>
            </span>
          </div>
        </Show>
      </section>
    </Show>
  );
}
