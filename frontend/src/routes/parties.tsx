import { addDays, dayOf, hm, localDate, monthOf, shortTime, today } from "~/lib/dates";
import { createAsync, query } from "@solidjs/router";
import { createEffect, For, Show } from "solid-js";
import { EntryRow } from "~/components/MemoryEntry";
import RecordDetail from "~/components/records/RecordDetail";
import RecordsPage, { type Group } from "~/components/records/RecordsPage";
import { CalendarMode } from "~/gen/famulen/v1/calendar_pb";
import { Speaker } from "~/gen/famulen/v1/console_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { workgroupsQuery } from "~/lib/data";
import { itemsOf, linkOf, type Rec, recordsQuery, valueOf, valuesOf, workgroupNames } from "~/lib/records";
import { usePicked } from "~/lib/picked";
import { useOwner } from "~/lib/owner";
import { workgroup } from "~/lib/workgroup";

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
    talk: talk.messages.filter((m) => says(m.text)).reverse().slice(0, 5),
  };
}, "partyAround");

/** What a party is to the owner. */
function toMe(r: Rec): string[] {
  return itemsOf(r, "Relations")
    .filter((i) => i.text === "me")
    .flatMap((i) => valuesOf(i, "As"));
}

function Avatar(props: { party: Rec; index: number }) {
  return (
    <span
      class={`avatar${valueOf(props.party, "Kind") === "person" || !valueOf(props.party, "Kind") ? "" : " org"}`}
      style={{ background: PALETTE[props.index % PALETTE.length] }}
    >
      {Array.from(props.party.title)[0] ?? "?"}
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
        <div class="list">
          <For each={memories() ?? []}>{(e) => <EntryRow item={e} />}</For>
          <Show when={memories()?.length === 0}>
            <p class="empty">まだ覚えていることはない</p>
          </Show>
        </div>
      </section>
      <Show when={(around()?.talk ?? []).length > 0}>
        <section class="rec-section">
          <h3>最近のやりとり</h3>
          <div class="list">
            <For each={around()?.talk}>
              {(m) => (
                <div class="item">
                  <span class="mono sub">{shortTime(m.at, around()?.tz ?? "UTC")}</span>
                  <span class="tag">{m.who === Speaker.OWNER ? "あなた" : aide()}</span>
                  <span class="grow" style={{ "font-size": "13px" }}>
                    {Array.from(m.text).slice(0, 120).join("")}
                  </span>
                </div>
              )}
            </For>
          </div>
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
            <button type="button" class="btn small go" onClick={() => act(() => api.records.answerAccount({ account: id(), same: true }), "同じ人にした")}>
              同じ人
            </button>
            <button type="button" class="btn small" onClick={() => act(() => api.records.answerAccount({ account: id(), same: false }), "外した")}>
              別の人
            </button>
          </div>
        );
      }}
    </For>
  );
}

export default function Parties() {
  const workgroups = createAsync(() => workgroupsQuery());
  const all = createAsync(() => recordsQuery("parties", workgroup()));
  usePicked({ kind: () => "parties", section: () => "相手" });
  const index = (r: Rec) => Math.max((all() ?? []).findIndex((x) => x.id === r.id), 0);
  const groups = (records: Rec[]): Group[] => {
    const sorted = [...records].sort((a, b) => a.title.localeCompare(b.title, "ja"));
    const of = (k: string) => sorted.filter((r) => (valueOf(r, "Kind") || "person") === k);
    return [
      { name: "人", records: of("person") },
      { name: "会社・チーム", records: of("org") },
      { name: "グループ", records: of("group") },
    ];
  };
  const body = (r: Rec) => {
    const places = () => itemsOf(r, "Member of").map((i) => linkOf(i.text)?.title).filter(Boolean);
    return (
      <>
        <Avatar party={r} index={index(r)} />
        <span class="rec-main">
          <b>{r.title}</b>
          <span class="rec-sub">
            <For each={toMe(r)}>{(t) => <span class="tag confirmed">{t}</span>}</For>
            <Show when={places().length > 0}>
              <span>{places().join("、")}</span>
            </Show>
            <Show when={valuesOf(r, "Aliases").length > 0}>
              <span>{valuesOf(r, "Aliases").join("、")}</span>
            </Show>
            <For each={r.labels}>{(l) => <span class="tag label">{l}</span>}</For>
            <span>{workgroupNames(workgroups(), r.workgroups)}</span>
          </span>
        </span>
      </>
    );
  };
  return (
    <RecordsPage
      kind="parties"
      base="/parties"
      title="相手"
      narrow={["Kind"]}
      groups={groups}
      row={body}
      card={body}
      detail={(id) => <RecordDetail kind="parties" id={id} back="/parties" top={(r) => <Guessed party={r} />} extra={(r) => <Around party={r} />} />}
      add={{ label: "グループを足す", titleLabel: "名前", fields: [], preset: { Kind: "group" } }}
      hint="人と会社・チームは、秘書がやりとりの中で覚えていく。"
      empty="相手はまだいない。秘書がやりとりの中で覚えていく。"
    />
  );
}
