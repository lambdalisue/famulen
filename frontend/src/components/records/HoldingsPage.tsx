import { Show, For } from "solid-js";
import { Tag } from "~/components/ui";
import { dateText, daysLeft, nearness } from "~/lib/dates";
import { useOwner } from "~/lib/owner";
import { usePicked } from "~/lib/picked";
import { choiceOf, deadlinesOf, excerptOf, isPast, linkOf, moneyText, type Rec, sectionOf, valueOf } from "~/lib/records";
import { byName } from "~/lib/words";
import RecordDetail from "./detail/RecordDetail";
import RecordsPage, { type Group } from "./RecordsPage";
import type { RowParts } from "./Row";

const next = (r: Rec) => (isPast(r) ? undefined : deadlinesOf(r)[0]);
const soonest = (r: Rec) => next(r)?.on ?? "9999";
const order = (a: Rec, b: Rec) => soonest(a).localeCompare(soonest(b)) || byName(a.title, b.title);

/** What a thing is and where it came from. */
function thingFacts(r: Rec): string[] {
  const got = sectionOf(r, "Acquired");
  const from = valueOf(got, "From");
  return [
    [valueOf(r, "Maker"), valueOf(r, "Model")].filter(Boolean).join(" "),
    valueOf(r, "Place"),
    valueOf(got, "On") ? `${dateText(valueOf(got, "On"))}${from ? ` ${linkOf(from)?.title ?? from}` : ""}` : "",
    moneyText(valueOf(got, "Price")),
  ];
}

/** What a contract is, and what it costs. */
function contractFacts(r: Rec): string[] {
  const price = valueOf(r, "Price");
  const cycle = valueOf(r, "Cycle");
  return [
    [valueOf(r, "Provider"), valueOf(r, "Plan")].filter(Boolean).join(" "),
    price ? `${moneyText(price)}${cycle ? ` / ${choiceOf(cycle)}` : ""}` : "",
    valueOf(r, "Auto renew") === "yes" ? "自動更新" : "",
  ];
}

const KINDS = {
  holdings: {
    title: "物",
    base: "/things",
    facts: thingFacts,
    overLabel: "手放した物も",
    add: { label: "物を足す", titleLabel: "名前", fields: ["Category", "Status", "Place"] },
    empty: "物はまだない",
  },
  contracts: {
    title: "契約",
    base: "/contracts",
    facts: contractFacts,
    overLabel: "終わった契約も",
    add: { label: "契約を足す", titleLabel: "名前", fields: ["Category", "Provider", "Price", "Cycle"] },
    empty: "契約はまだない",
  },
};

/** By category, those with the soonest deadline first. */
function groups(records: Rec[]): Group[] {
  const by = new Map<string, Rec[]>();
  const sorted = [...records].sort(order);
  for (const r of sorted) {
    const c = valueOf(r, "Category") || "分類なし";
    by.set(c, [...(by.get(c) ?? []), r]);
  }
  if (by.size <= 1) return [{ name: "", records: sorted }];
  return [...by]
    .sort(([a, ra], [b, rb]) => soonest(ra[0]).localeCompare(soonest(rb[0])) || byName(a, b))
    .map(([name, rs]) => ({ name, records: rs }));
}

/** Things, or contracts: the same page over a different kind. */
export default function HoldingsPage(props: { kind: "holdings" | "contracts" }) {
  const of = () => KINDS[props.kind];
  const owner = useOwner();
  usePicked({ kind: () => props.kind, section: () => of().title });
  const row = (r: () => Rec): RowParts => ({
    sub: () => (
      <>
        <Show when={valueOf(r(), "Status") && !["owned", "active"].includes(valueOf(r(), "Status"))}>
          <Tag>{choiceOf(valueOf(r(), "Status"))}</Tag>
        </Show>
        <For each={of().facts(r()).filter(Boolean)}>{(f) => <span>{f}</span>}</For>
      </>
    ),
    end: () => (
      <Show when={next(r())}>
        {(d) => (
          <span class="rec-side">
            <span class={`rec-when ${nearness(d().on, owner.today())}`}>
              {d().what} {daysLeft(d().on, owner.today())}
            </span>
            <span class="sub">{dateText(d().on)}</span>
          </span>
        )}
      </Show>
    ),
    excerpt: () => excerptOf(r().notes),
  });
  return (
    <RecordsPage
      kind={props.kind}
      base={of().base}
      title={of().title}
      narrow={["Category", "Status"]}
      over={isPast}
      overLabel={of().overLabel}
      groups={groups}
      row={row}
      cards
      detail={(id) => <RecordDetail kind={props.kind} id={id} back={of().base} />}
      add={of().add}
      hint="期限の 2 週間前になると、タスクになる。"
      empty={of().empty}
    />
  );
}
