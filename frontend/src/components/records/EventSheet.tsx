import { Dialog } from "@ark-ui/solid/dialog";
import { A, createAsync, useNavigate } from "@solidjs/router";
import { For, Show } from "solid-js";
import Icon from "~/components/Icon";
import Markdown from "~/components/Markdown";
import SideSheet from "~/components/SideSheet";
import type { CalendarEvent } from "~/gen/famulen/v1/calendar_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { ask } from "~/lib/ask";
import { addDays, dateLabel, hm, localDate } from "~/lib/calendar";
import * as icons from "~/lib/icons";
import { choiceOf, hrefOf } from "~/lib/records";
import RecordDetail from "./RecordDetail";
import "./records.css";

/** When an event is, in words. */
export function whenOf(e: CalendarEvent): string {
  const first = localDate(e.start);
  const last = e.allDay ? addDays(localDate(e.end), -1) : localDate(e.end);
  if (e.allDay) return first === last ? `${dateLabel(first)} 終日` : `${dateLabel(first)} – ${dateLabel(last)} 終日`;
  return first === last ? `${dateLabel(first)} ${hm(e.start)} – ${hm(e.end)}` : `${dateLabel(first)} ${hm(e.start)} – ${dateLabel(last)} ${hm(e.end)}`;
}

const MARK: { [response: string]: string } = { accepted: "出席", declined: "欠席", tentative: "仮", needs_action: "未回答" };

/** Where an event leads: its meeting, its map, its page in Google. */
function EventButtons(props: { event: CalendarEvent }) {
  const e = () => props.event;
  return (
    <div class="row">
      <Show when={e().meeting}>
        <a class="btn primary" href={e().meeting} target="_blank" rel="noreferrer">
          <Icon svg={icons.video} size={15} />
          会議に入る
        </a>
      </Show>
      <Show when={e().map}>
        <a class="btn" href={e().map} target="_blank" rel="noreferrer">
          <Icon svg={icons.mapPin} size={15} />
          地図
        </a>
      </Show>
      <Show when={e().url}>
        <a class="btn" href={e().url} target="_blank" rel="noreferrer">
          <Icon svg={icons.externalLink} size={15} />
          Google で開く
        </a>
      </Show>
    </div>
  );
}

function Attendees(props: { event: CalendarEvent }) {
  const people = () => props.event.attendees;
  const count = (r: string) => people().filter((a) => a.response === r).length;
  return (
    <Show when={people().length > 0}>
      <section class="rec-section">
        <h3>
          参加者 {people().length} 人
          <span class="sub">
            {" "}
            出席 {count("accepted")} · 欠席 {count("declined")} · 未回答 {count("needs_action")}
          </span>
        </h3>
        <ul class="ev-people">
          <For each={people()}>
            {(a) => (
              <li>
                <span class="ev-answer" data-answer={a.response}>
                  {MARK[a.response] ?? a.response}
                </span>
                <Show when={a.party} fallback={<span class="grow">{a.name || a.email}</span>}>
                  <A class="grow" href={hrefOf("parties", a.party) ?? ""}>
                    {a.name || a.email}
                  </A>
                </Show>
                <Show when={a.organizer}>
                  <span class="tag">主催</span>
                </Show>
                <Show when={a.optional}>
                  <span class="tag">任意</span>
                </Show>
              </li>
            )}
          </For>
        </ul>
      </section>
    </Show>
  );
}

/** One event, with everything it says; a sheet from the right. */
export default function EventSheet(props: { event: CalendarEvent | undefined; record: string; onClose: () => void; back: string }) {
  const navigate = useNavigate();
  const e = () => props.event;
  /** The record that holds the event, or that adds to one in Google. */
  const own = () => (e() && !e()?.google ? e()?.record : undefined) || props.record;
  const addNotes = async (event: CalendarEvent) => {
    const made = await act(
      () => api.records.saveRecord({ kind: "events", title: event.title, google: event.series || event.uid, url: event.url }),
      "書き足せるようにした",
    );
    if (made) navigate(`${props.back}${props.back.includes("?") ? "&" : "?"}record=${made.id}`);
  };
  return (
    <Dialog.Root open={!!e() || !!props.record}
      onOpenChange={(d) => !d.open && props.onClose()}
      lazyMount
      unmountOnExit
    >
      <SideSheet name="event" title={e()?.title || "予定"}>
        <Show when={e()}>
          {(event) => (
            <article class="rec ev-detail">
              <div class="row">
                <span class="dot" style={{ background: event().color || "#0F6E68" }} />
                <b>{whenOf(event())}</b>
                <Show when={event().repeats}>
                  <Icon svg={icons.repeat} size={14} />
                </Show>
              </div>
              <div class="chips-line">
                <Show when={event().calendarName}>
                  <span class="tag">{event().calendarName}</span>
                </Show>
                <Show when={event().status !== "confirmed"}>
                  <span class="tag warn">{choiceOf(event().status)}</span>
                </Show>
                <Show when={event().response}>
                  <span class="tag">あなたは{choiceOf(event().response)}</span>
                </Show>
                <Show when={!event().busy}>
                  <span class="tag">時間はふさがない</span>
                </Show>
                <For each={event().labels}>{(l) => <span class="tag label">{l}</span>}</For>
              </div>
              <EventButtons event={event()} />
              <Show when={event().location}>
                <div class="callout">
                  <Icon svg={icons.mapPin} size={15} />
                  <span class="grow">{event().location}</span>
                </div>
              </Show>
              <Show when={event().stale}>
                <div class="callout warn">Google を読めなかったので、前に読んだ中身を出している。</div>
              </Show>
              <Attendees event={event()} />
              <Show when={event().google && event().notes}>
                <section class="rec-section">
                  <h3>備考</h3>
                  <Markdown text={event().notes} />
                </section>
              </Show>
              <Show when={event().google}>
                <section class="rec-section">
                  <div class="row">
                    <Show
                      when={event().record}
                      fallback={
                        <button type="button" class="btn small" onClick={() => void addNotes(event())}>
                          <Icon svg={icons.pencil} size={13} />
                          備考やラベルを書き足す
                        </button>
                      }
                    >
                      <A class="btn small" href={`${props.back}${props.back.includes("?") ? "&" : "?"}record=${event().record}`}>
                        <Icon svg={icons.pencil} size={13} />
                        書き足したものを開く
                      </A>
                    </Show>
                    <button
                      type="button"
                      class="btn small"
                      onClick={() => {
                        props.onClose();
                        queueMicrotask(() => ask(`「${event().title}」（${whenOf(event())}）を`));
                      }}
                    >
                      変更を頼む
                    </button>
                  </div>
                  <p class="sub">書き足したものは手帳に置かれ、Google には送らない。</p>
                </section>
              </Show>
            </article>
          )}
        </Show>
        <Show when={own()}>{(id) => <RecordDetail kind="events" id={id()} back={props.back} />}</Show>
      </SideSheet>
    </Dialog.Root>
  );
}
