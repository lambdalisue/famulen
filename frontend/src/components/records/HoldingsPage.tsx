import { dateText, daysLeft , nearness } from "~/lib/dates";
import { createAsync } from "@solidjs/router";
import { createEffect, For, Show } from "solid-js";
import { workgroupsQuery } from "~/lib/data";
import { choiceOf, deadlinesOf, excerptOf, isPast, linkOf, moneyText, type Rec, sectionOf, valueOf, workgroupNames } from "~/lib/records";
import { usePicked } from "~/lib/picked";
import RecordDetail from "./RecordDetail";
import { useOwner } from "~/lib/owner";
import RecordsPage, { type Group } from "./RecordsPage";

/** Things, or contracts: the same page over a different kind. */
export default function HoldingsPage(props: { kind: "holdings" | "contracts" }) {
  const things = () => props.kind === "holdings";
  const title = () => (things() ? "物" : "契約");
  const base = () => (things() ? "/things" : "/contracts");
  const owner = useOwner();
  const workgroups = createAsync(() => workgroupsQuery());
  const day = owner.today;
  usePicked({ kind: () => props.kind, section: title });

  const next = (r: Rec) => (isPast(r) ? undefined : deadlinesOf(r)[0]);
  const order = (a: Rec, b: Rec) => (next(a)?.on ?? "9999").localeCompare(next(b)?.on ?? "9999") || a.title.localeCompare(b.title, "ja");
  const groups = (records: Rec[]): Group[] => {
    const by = new Map<string, Rec[]>();
    for (const r of [...records].sort(order)) {
      const c = valueOf(r, "Category") || "分類なし";
      by.set(c, [...(by.get(c) ?? []), r]);
    }
    if (by.size <= 1) return [{ name: "", records: [...records].sort(order) }];
    const soon = (rs: Rec[]) => next(rs[0])?.on ?? "9999";
    return [...by]
      .sort(([a, ra], [b, rb]) => soon(ra).localeCompare(soon(rb)) || a.localeCompare(b, "ja"))
      .map(([name, rs]) => ({ name, records: rs }));
  };
  /** What a thing is and where it came from, or what a contract is and costs. */
  const facts = (r: Rec): string[] => {
    const text = (name: string) => valueOf(r, name);
    const got = sectionOf(r, "Acquired");
    const from = valueOf(got, "From");
    return things()
      ? [
          [text("Maker"), text("Model")].filter(Boolean).join(" "),
          text("Place"),
          valueOf(got, "On") ? `${dateText(valueOf(got, "On"))}${from ? ` ${linkOf(from)?.title ?? from}` : ""}` : "",
          moneyText(valueOf(got, "Price")),
        ]
      : [
          [text("Provider"), text("Plan")].filter(Boolean).join(" "),
          text("Price") ? `${moneyText(text("Price"))}${text("Cycle") ? ` / ${choiceOf(text("Cycle"))}` : ""}` : "",
          text("Auto renew") === "yes" ? "自動更新" : "",
        ];
  };
  const body = (r: Rec, card: boolean) => (
    <>
      <span class="rec-main">
        <b>{r.title}</b>
        <span class="rec-sub">
          <Show when={valueOf(r, "Status") && !["owned", "active"].includes(valueOf(r, "Status"))}>
            <span class="tag">{choiceOf(valueOf(r, "Status"))}</span>
          </Show>
          <Show when={card && valueOf(r, "Category")}>
            <span class="tag">{valueOf(r, "Category")}</span>
          </Show>
          <For each={facts(r).filter(Boolean)}>{(f) => <span>{f}</span>}</For>
          <For each={r.labels}>{(l) => <span class="tag label">{l}</span>}</For>
          <span>{workgroupNames(workgroups(), r.workgroups)}</span>
        </span>
        <Show when={card && r.notes}>
          <span class="rec-excerpt">{excerptOf(r.notes)}</span>
        </Show>
      </span>
      <Show when={next(r)}>
        {(d) => (
          <span class="rec-side">
            <span class="rec-when" classList={{ late: nearness(d().on, day()) === "late", soon: nearness(d().on, day()) === "soon" }}>
              {d().what} {daysLeft(d().on, day())}
            </span>
            <span class="sub mono">{d().on}</span>
          </span>
        )}
      </Show>
    </>
  );
  return (
    <RecordsPage
      kind={props.kind}
      base={base()}
      title={title()}
      narrow={["Category", "Status"]}
      over={isPast}
      overLabel={things() ? "手放した物も" : "終わった契約も"}
      groups={groups}
      row={(r) => body(r, false)}
      card={(r) => body(r, true)}
      detail={(id) => <RecordDetail kind={props.kind} id={id} back={base()} />}
      add={
        things()
          ? { label: "物を足す", titleLabel: "名前", fields: ["Category", "Status", "Place"] }
          : { label: "契約を足す", titleLabel: "名前", fields: ["Category", "Provider", "Price", "Cycle"] }
      }
      hint="期限の 2 週間前になると、タスクになる。"
      empty={things() ? "物はまだない。" : "契約はまだない。"}
    />
  );
}
