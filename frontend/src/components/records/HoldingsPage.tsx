import { createSignal, For, onMount, Show } from "solid-js";
import { openImage } from "~/components/Lightbox";
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

const categoryOf = (r: Rec) => valueOf(r, "Category") || "分類なし";

const KINDS = {
  holdings: {
    title: "物",
    narrow: ["Status"],
    tabs: { label: "分類", param: "category", field: "Category", of: categoryOf },
    base: "/things",
    facts: thingFacts,
    overLabel: "手放した物も",
    add: { label: "物を足す", titleLabel: "名前", fields: ["Category", "Status", "Place"] },
    empty: "物はまだない",
  },
  contracts: {
    title: "契約",
    narrow: ["Category", "Status"],
    tabs: undefined,
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
    const c = categoryOf(r);
    by.set(c, [...(by.get(c) ?? []), r]);
  }
  if (by.size <= 1) return [{ name: "", records: sorted }];
  return [...by]
    .sort(([a, ra], [b, rb]) => soonest(ra[0]).localeCompare(soonest(rb[0])) || byName(a, b))
    .map(([name, rs]) => ({ name, records: rs }));
}

/** Whether a thing's picture is to be shown: it is not when it could not be read. */
function useShown(r: () => Rec) {
  const [missing, setMissing] = createSignal("");
  let img: HTMLImageElement | undefined;
  onMount(() => {
    // It may have failed before this code ran.
    if (img?.complete && img.naturalWidth === 0) setMissing(r().picture);
  });
  return {
    url: () => (r().picture && r().picture !== missing() ? `/holding-pictures/${encodeURIComponent(r().picture)}` : ""),
    ref: (el: HTMLImageElement) => {
      img = el;
    },
    onError: () => setMissing(r().picture),
  };
}

function Thumbnail(props: { thing: Rec }) {
  const shown = useShown(() => props.thing);
  return (
    <Show when={shown.url()}>
      {(url) => <img ref={shown.ref} class="thing-thumb" src={url()} alt="" loading="lazy" onError={shown.onError} />}
    </Show>
  );
}

function Picture(props: { thing: Rec }) {
  const shown = useShown(() => props.thing);
  return (
    <Show when={shown.url()}>
      {(url) => (
        <button
          type="button"
          class="thing-picture"
          aria-label={`${props.thing.title}の画像を拡大する`}
          aria-haspopup="dialog"
          onClick={(e) => openImage({ name: props.thing.title, url: url() }, e.currentTarget)}
        >
          <img ref={shown.ref} src={url()} alt={props.thing.title} onError={shown.onError} />
        </button>
      )}
    </Show>
  );
}

/** Things, or contracts: the same page over a different kind. */
export default function HoldingsPage(props: { kind: "holdings" | "contracts" }) {
  const of = () => KINDS[props.kind];
  const owner = useOwner();
  usePicked({ kind: () => props.kind, section: () => of().title });
  const row = (r: () => Rec): RowParts => ({
    lead: () => <Thumbnail thing={r()} />,
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
      narrow={of().narrow}
      tabs={of().tabs}
      over={isPast}
      overLabel={of().overLabel}
      groups={groups}
      row={row}
      cards
      detail={(id, back) => <RecordDetail kind={props.kind} id={id} back={back()} top={(r) => <Picture thing={r()} />} />}
      add={of().add}
      hint="期限の 2 週間前になると、タスクになる。"
      empty={of().empty}
    />
  );
}
