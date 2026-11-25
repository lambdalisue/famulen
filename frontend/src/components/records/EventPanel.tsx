import { addDays, dateLabel, hm, localDate } from "~/lib/dates";
import { createAsync, useNavigate } from "@solidjs/router";
import { For, type JSX, Show } from "solid-js";
import AskAbout from "~/components/AskAbout";
import Markdown from "~/components/Markdown";
import type { CalendarEvent } from "~/gen/famulen/v1/calendar_pb";
import { withParams } from "~/lib/params";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { ask } from "~/lib/ask";
import * as icons from "~/lib/icons";
import { choiceOf, hrefOf } from "~/lib/records";
import RecordDetail from "./detail/RecordDetail";
import { Button, Callout, Icon, LinkButton, Tag } from "~/components/ui";

/** When an event is, in words. */
export function whenOf(e: CalendarEvent): string {
  const first = localDate(e.start);
  const last = e.allDay ? addDays(localDate(e.end), -1) : localDate(e.end);
  if (e.allDay) return first === last ? `${dateLabel(first)} 終日` : `${dateLabel(first)} – ${dateLabel(last)} 終日`;
  return first === last
    ? `${dateLabel(first)} ${hm(e.start)} – ${hm(e.end)}`
    : `${dateLabel(first)} ${hm(e.start)} – ${dateLabel(last)} ${hm(e.end)}`;
}

const MARK: { [response: string]: string } = { accepted: "出席", declined: "欠席", tentative: "仮", needs_action: "未回答" };

/** How long an event runs, in words; nothing for whole days. */
function lengthOf(e: CalendarEvent): string {
  const minutes = Math.round((Date.parse(e.end) - Date.parse(e.start)) / 60_000);
  if (e.allDay || !(minutes > 0)) return "";
  const [h, m] = [Math.floor(minutes / 60), minutes % 60];
  return [h ? `${h} 時間` : "", m ? `${m} 分` : ""].filter(Boolean).join(" ");
}

/** One thing about an event: what it is by its icon, then what it says. */
function Fact(props: { icon: string; label: string; children: JSX.Element; action?: JSX.Element }) {
  return (
    <div class="ev-fact">
      <Icon svg={props.icon} size={18} />
      <dt class="sr">{props.label}</dt>
      <dd>{props.children}</dd>
      {props.action}
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
            {(["accepted", "declined", "tentative", "needs_action"] as const)
              .filter((r) => count(r) > 0)
              .map((r) => `${MARK[r]} ${count(r)}`)
              .join(" · ")}
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
                  <a class="grow" href={hrefOf("parties", a.party) ?? ""}>
                    {a.name || a.email}
                  </a>
                </Show>
                <Show when={a.organizer}>
                  <Tag>主催</Tag>
                </Show>
                <Show when={a.optional}>
                  <Tag>任意</Tag>
                </Show>
              </li>
            )}
          </For>
        </ul>
      </section>
    </Show>
  );
}

/** When and where an event is, who comes, and where it leads. */
function Facts(props: { event: CalendarEvent }) {
  const e = () => props.event;
  const marks = () => e().status !== "confirmed" || !e().busy || (e().google && e().labels.length > 0);
  return (
    <>
      <Show when={e().meeting}>
        <LinkButton variant="go" class="ev-join" icon={icons.video} href={e().meeting}>
          会議に入る
        </LinkButton>
      </Show>
      <Show when={e().stale}>
        <Callout tone="warn">Google を読めなかったので、前に読んだ中身を出している。</Callout>
      </Show>
      <dl class="ev-facts" style={{ "--c": e().color || undefined }}>
        <Fact icon={icons.clock} label="日時">
          <b>{whenOf(e())}</b>
          <span class="sub">
            {[lengthOf(e()), e().repeats ? "繰り返す" : ""].filter(Boolean).join(" · ")}
          </span>
        </Fact>
        <Show when={e().location || e().map}>
          <Fact
            icon={icons.mapPin}
            label="場所"
            action={
              <Show when={e().map}>
                <LinkButton small icon={icons.externalLink} href={e().map}>
                  地図
                </LinkButton>
              </Show>
            }
          >
            {e().location || "地図がある"}
          </Fact>
        </Show>
        <Show when={e().response}>
          <Fact icon={icons.userCheck} label="あなたの出欠">
            <span class="ev-answer" data-answer={e().response}>
              {MARK[e().response] ?? choiceOf(e().response)}
            </span>
          </Fact>
        </Show>
        <Show when={e().calendarName || e().url}>
          <Fact
            icon={icons.calendar}
            label="カレンダー"
            action={
              <Show when={e().url}>
                <LinkButton small icon={icons.externalLink} href={e().url}>
                  Google で開く
                </LinkButton>
              </Show>
            }
          >
            <span class="dot" style={{ background: e().color || "var(--accent)" }} /> {e().calendarName || "Google"}
          </Fact>
        </Show>
        <Show when={marks()}>
          <Fact icon={icons.tag} label="印">
            <div class="chips-line">
              <Show when={e().status !== "confirmed"}>
                <Tag tone="warn">{choiceOf(e().status)}</Tag>
              </Show>
              <Show when={!e().busy}>
                <Tag>時間はふさがない</Tag>
              </Show>
              <Show when={e().google}>
                <For each={e().labels}>{(l) => <Tag tone="label">{l}</Tag>}</For>
              </Show>
            </div>
          </Fact>
        </Show>
      </dl>
      <Attendees event={e()} />
    </>
  );
}

/** An event of Google's, which is read here and changed there, with what the owner may add to it. */
function FromGoogle(props: { event: CalendarEvent; back: string }) {
  const navigate = useNavigate();
  const e = () => props.event;
  const addNotes = async () => {
    const made = await act(
      () => api.records.saveRecord({ kind: "events", title: e().title, google: e().series || e().uid, url: e().url }),
      "書き足せるようにした",
    );
    if (made) navigate(withParams(props.back, { record: made.id }));
  };
  return (
    <article class="rec event">
      <h2 class="event-title" style={{ "--c": e().color || undefined }}>
        {e().title || "（題名なし）"}
      </h2>
      <Facts event={e()} />
      <Show when={e().notes}>
        <section class="rec-section">
          <h3>備考</h3>
          <div class="rec-prose">
            <Markdown text={e().notes} />
          </div>
        </section>
      </Show>
      <section class="rec-section">
        <div class="row">
          <Show
            when={e().record}
            fallback={
              <Button small icon={icons.pencil} onClick={() => void addNotes()}>
                備考やラベルを書き足す
              </Button>
            }
          >
            <LinkButton small icon={icons.pencil} href={withParams(props.back, { record: e().record })}>
              書き足したものを開く
            </LinkButton>
          </Show>
          <AskAbout framed what={`予定：${whenOf(e())} ${e().title || "（題名なし）"}`} />
          <Button small onClick={() => ask(`「${e().title}」（${whenOf(e())}）を`)}>
            変更を頼む
          </Button>
        </div>
        <p class="sub">書き足したものは手帳に置かれ、Google には送らない。</p>
      </section>
    </article>
  );
}

/** One event, with everything it says. */
export default function EventPanel(props: { event: CalendarEvent | undefined; record: string; back: string }) {
  /** The record that holds the event, or that adds to one in Google. */
  const own = () => (props.event && !props.event.google ? props.event.record : undefined) || props.record;
  return (
    <>
      <Show when={props.event?.google ? props.event : undefined}>{(e) => <FromGoogle event={e()} back={props.back} />}</Show>
      <Show when={own()} keyed>
        {(id) => (
          <RecordDetail
            kind="events"
            id={id}
            back={props.back}
            fold={props.event && !props.event.google ? "日時や場所を直す" : undefined}
            top={() => <Show when={props.event && !props.event.google ? props.event : undefined}>{(e) => <Facts event={e()} />}</Show>}
          />
        )}
      </Show>
    </>
  );
}
