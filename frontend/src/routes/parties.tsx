import { addDays, dayOf, hm, localDate, monthOf, shortTime, today } from "~/lib/dates";
import { createAsync, query } from "@solidjs/router";
import { createEffect, createSignal, For, onMount, Show } from "solid-js";
import { EntryRow } from "~/components/MemoryEntry";
import RecordDetail from "~/components/records/detail/RecordDetail";
import RecordsPage, { type Group } from "~/components/records/RecordsPage";
import type { RowParts } from "~/components/records/Row";
import { CalendarMode } from "~/gen/famulen/v1/calendar_pb";
import { Speaker } from "~/gen/famulen/v1/console_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { isResting, itemsOf, linkOf, type Rec, recordsQuery, valueOf, valuesOf } from "~/lib/records";
import { usePicked } from "~/lib/picked";
import { useOwner } from "~/lib/owner";
import { byName } from "~/lib/words";
import { workgroup } from "~/lib/workgroup";
import { Button, Callout, EmptyState, List, Tag } from "~/components/ui";

const PALETTE = ["#0F6E68", "#B4610E", "#5B5FA8", "#8A5A7A", "#3B3A36"];

const partyMemoriesQuery = query(async (id: string) => (await api.memory.listPartyMemories({ id })).items, "partyMemories");

/** What is coming up with a party, and what was said of it. */
const partyAroundQuery = query(async (workgroup: string, id: string, names: string) => {
  const tz = (await api.settings.getPersona({})).timezone;
  const from = today(tz);
  const called = names.split("\n").map((n) => n.replace(/さん$/, ""));
  const [events, talk] = await Promise.all([
    api.calendar.listEvents({ workgroup, from, to: addDays(from, 30) }),
    api.console.listMessages({ workgroup: "all", limit: 300 }),
  ]);
  const says = (text: string) => called.some((n) => n.length > 1 && text.includes(n));
  return {
    tz,
    events: events.events.filter((e) => e.mode === CalendarMode.SHOW && (e.attendees.some((a) => a.party === id) || says(e.title))),
    talk: talk.messages
      .filter((m) => says(m.text))
      .reverse()
      .slice(0, 5),
  };
}, "partyAround");

/** What a party is to the owner. */
function toMe(r: Rec): string[] {
  return itemsOf(r, "Relations")
    .filter((i) => i.text === "me")
    .flatMap((i) => valuesOf(i, "As"));
}

/** A party's picture, or the start of its name. */
function Avatar(props: { party: Rec; index: number }) {
  const [missing, setMissing] = createSignal("");
  const shown = () => (props.party.picture === missing() ? "" : props.party.picture);
  let img: HTMLImageElement | undefined;
  onMount(() => {
    // It may have failed before this code ran.
    if (img?.complete && img.naturalWidth === 0) setMissing(props.party.picture);
  });
  return (
    <span
      class={`avatar${valueOf(props.party, "Kind") === "person" || !valueOf(props.party, "Kind") ? "" : " org"}`}
      style={{ background: PALETTE[props.index % PALETTE.length] }}
    >
      <Show when={shown()} fallback={Array.from(props.party.title)[0] ?? "?"}>
        <img
          ref={img}
          src={`/party-pictures/${encodeURIComponent(shown())}`}
          alt=""
          onError={() => setMissing(props.party.picture)}
        />
      </Show>
    </span>
  );
}

/** The accounts still to be confirmed, and what memory and the days ahead hold. */
function Around(props: { party: Rec }) {
  const p = () => props.party;
  const owner = useOwner();
  const aide = owner.aide;
  const around = createAsync(() => partyAroundQuery(workgroup(), p().id, [p().title, ...valuesOf(p(), "Aliases")].join("\n")));
  const memories = createAsync(() => partyMemoriesQuery(p().id));
  return (
    <>
      <Show when={(around()?.events ?? []).length > 0}>
        <section class="rec-section">
          <h3>これからの予定</h3>
          <div class="chips-line">
            <For each={around()?.events}>
              {(e) => (
                <a class="tag link" href={`/calendar?unit=day&at=${localDate(e.start)}&event=${encodeURIComponent(e.uid)}`}>
                  {monthOf(localDate(e.start))}/{dayOf(localDate(e.start))} {e.allDay ? "" : hm(e.start)} {e.title}
                </a>
              )}
            </For>
          </div>
        </section>
      </Show>
      <section class="rec-section">
        <h3>{aide()} が覚えていること</h3>
        <List>
          <For each={memories() ?? []}>{(e) => <EntryRow item={e} />}</For>
          <Show when={memories()?.length === 0}>
            <EmptyState>まだ覚えていることはない</EmptyState>
          </Show>
        </List>
      </section>
      <Show when={(around()?.talk ?? []).length > 0}>
        <section class="rec-section">
          <h3>最近のやりとり</h3>
          <List>
            <For each={around()?.talk}>
              {(m) => (
                <div role="listitem" class="item">
                  <span class="mono sub">{shortTime(m.at, around()?.tz ?? "UTC")}</span>
                  <Tag>{m.who === Speaker.OWNER ? "あなた" : aide()}</Tag>
                  <span class="grow">{Array.from(m.text).slice(0, 120).join("")}</span>
                </div>
              )}
            </For>
          </List>
        </section>
      </Show>
    </>
  );
}

function Guessed(props: { party: Rec }) {
  const guessed = () => itemsOf(props.party, "Accounts").filter((i) => valueOf(i, "Confirmed") !== "yes");
  return (
    <For each={guessed()}>
      {(a) => {
        const id = () => `${a.text}:${valueOf(a, "Handle")}`;
        return (
          <div class="callout warn">
            <span class="grow">
              {a.text} の <b class="mono">{valueOf(a, "Handle")}</b> は同じ相手か（{valueOf(a, "How")}）
            </span>
            <Button variant="go" small onClick={() => act(() => api.records.answerAccount({ account: id(), same: true }), "同じ人にした")}>
              同じ人
            </Button>
            <Button small onClick={() => act(() => api.records.answerAccount({ account: id(), same: false }), "外した")}>
              別の人
            </Button>
          </div>
        );
      }}
    </For>
  );
}

/** People, companies and groups, each by name, and whoever rests put away below. */
function groups(records: Rec[]): Group[] {
  const sorted = [...records].sort((a, b) => byName(a.title, b.title));
  const of = (k: string) => sorted.filter((r) => !isResting(r) && (valueOf(r, "Kind") || "person") === k);
  return [
    { name: "人", records: of("person") },
    { name: "会社・チーム", records: of("org") },
    { name: "グループ", records: of("group") },
    { name: "休み", records: sorted.filter(isResting), folded: true },
  ];
}

const places = (r: Rec) =>
  itemsOf(r, "Member of")
    .map((i) => linkOf(i.text)?.title)
    .filter(Boolean);

export default function Parties() {
  const all = createAsync(() => recordsQuery("parties", workgroup()));
  usePicked({ kind: () => "parties", section: () => "相手" });
  const index = (r: Rec) =>
    Math.max(
      (all() ?? []).findIndex((x) => x.id === r.id),
      0,
    );
  const row = (r: () => Rec): RowParts => ({
    lead: () => <Avatar party={r()} index={index(r())} />,
    sub: () => (
      <>
        <For each={toMe(r())}>{(t) => <Tag tone="ok">{t}</Tag>}</For>
        <Show when={places(r()).length > 0}>
          <span>{places(r()).join("、")}</span>
        </Show>
        <Show when={valuesOf(r(), "Aliases").length > 0}>
          <span>{valuesOf(r(), "Aliases").join("、")}</span>
        </Show>
      </>
    ),
  });
  return (
    <RecordsPage
      kind="parties"
      base="/parties"
      title="相手"
      narrow={["Kind"]}
      groups={groups}
      row={row}
      cards
      detail={(id) => (
        <RecordDetail
          kind="parties"
          id={id}
          back="/parties"
          lead={(r) => <Avatar party={r} index={index(r)} />}
          top={(r) => <Guessed party={r()} />}
          extra={(r) => <Around party={r()} />}
        />
      )}
      add={{ label: "グループを足す", titleLabel: "名前", fields: [], preset: { Kind: "group" } }}
      hint="人と会社・チームは、秘書がやりとりの中で覚えていく。長く出てこない人は休みになり、また出ると戻る。"
      empty="相手はまだいない"
    />
  );
}
