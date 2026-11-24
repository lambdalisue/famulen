import { createAsync, query } from "@solidjs/router";
import { createMemo, createSignal, For, Show } from "solid-js";
import type { LogEvent, LogItem, LogTurn } from "~/gen/famulen/v1/settings_pb";
import { clock, dayLabel } from "~/lib/dates";
import { api } from "~/lib/api";
import * as icons from "~/lib/icons";
import { setSelection } from "~/lib/view";
import { useOwner } from "~/lib/owner";
import { createPages, PAGE, PageNav } from "./Pages";
import { EmptyState, Icon, SegmentedOf, Tag } from "~/components/ui";

const logQuery = query(async (show: string, before: number) => {
  const page = await api.log.listLog({ show, before: BigInt(before), limit: PAGE });
  return { rows: page.items, more: page.more };
}, "log");

const FILTERS: [string, string][] = [
  ["", "すべて"],
  ["talk", "秘書のやりとり"],
  ["other", "夢・見張り・設定"],
];

const EVENTS: Record<string, string> = {
  dream: "夢",
  setting: "設定",
  lookout: "見張り",
  schedule: "定期の仕事",
  wake: "起床予約",
  error: "うまくいかなかった",
};

function atOf(item: LogItem): bigint {
  return item.item.case ? item.item.value.at : 0n;
}

/** What the secretary did, turn by turn, and what happened around it. */
export default function SecretaryLog() {
  const [show, setShow] = createSignal("");
  const pages = createPages((before) => logQuery(show(), before));
  const items = pages.rows;
  const choose = (key: string) => {
    setShow(key);
    pages.reset();
  };
  const owner = useOwner();
  const tz = owner.tz;

  const days = createMemo(() => {
    const groups: { day: string; items: LogItem[] }[] = [];
    for (const item of items()) {
      const day = dayLabel(atOf(item), tz());
      const last = groups.at(-1);
      if (last?.day === day) last.items.push(item);
      else groups.push({ day, items: [item] });
    }
    return groups;
  });

  return (
    <>
      <SegmentedOf label="出すもの" items={FILTERS} value={show()} onChoose={choose} />
      <div class="stack">
        <For each={days()}>
          {(g) => (
            <>
              <div class="log-day">{g.day}</div>
              <For each={g.items}>
                {(item) => {
                  const it = item.item;
                  if (it.case === "turn") return <Turn turn={it.value} tz={tz()} />;
                  if (it.case === "event") return <Event event={it.value} tz={tz()} />;
                  return null;
                }}
              </For>
            </>
          )}
        </For>
        <Show when={pages.loaded() && items().length === 0}>
          <EmptyState>まだない</EmptyState>
        </Show>
      </div>
      <PageNav pages={pages} />
    </>
  );
}

function Turn(props: { turn: LogTurn; tz: string }) {
  const [open, setOpen] = createSignal(false);
  const t = () => props.turn;
  const stopped = () => t().steps.filter((s) => s.outcome !== "");
  const reply = () => t().reply.replaceAll("**", "");
  const toggle = () => {
    const next = !open();
    setOpen(next);
    if (next) setSelection([...`ログ · ${clock(t().at, props.tz)} ${t().asked || reply()}`].slice(0, 120).join(""));
  };

  return (
    <section
      class={`card log-turn${open() ? " open" : ""}`}
      role="button"
      tabindex="0"
      aria-expanded={open()}
      onClick={(e) => {
        if (window.getSelection()?.toString()) return;
        if (e.target instanceof HTMLElement && e.target.closest("a")) return;
        toggle();
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          toggle();
        }
      }}
    >
      <div class="row">
        <span class="sub mono">{clock(t().at, props.tz)}</span>
        <Show when={t().origin}>
          <Tag>{t().origin}から</Tag>
        </Show>
        <Show when={t().steps.length > 0}>
          <Tag>道具 {t().steps.length} 回</Tag>
        </Show>
        <For each={stopped()}>
          {(s) => (
            <Tag tone="warn">
              {s.tool} · {s.outcome}
            </Tag>
          )}
        </For>
        <span class="sub push">
          <Icon svg={open() ? icons.chevronDown : icons.chevronRight} size={14} />
        </span>
      </div>
      <Show when={t().asked}>
        <div class="asked">{t().asked}</div>
      </Show>
      <Show when={!open() && t().steps.length > 0}>
        <div class="log-steps">
          <For each={t().steps}>
            {(s) => (
              <Tag>
                <Icon svg={icons.plugConnected} size={12} />
                {s.service ? `${s.service} · ` : ""}
                {s.tool}
              </Tag>
            )}
          </For>
        </div>
      </Show>
      <Show when={open() && t().steps.length > 0}>
        <div class="log-step-detail">
          <For each={t().steps}>
            {(s) => (
              <div>
                <div class="sub">
                  <Icon svg={icons.plugConnected} size={12} /> {s.service ? `${s.service} · ` : ""}
                  {s.tool}
                  <Show when={s.outcome}>
                    {" "}
                    <Tag tone="warn">{s.outcome}</Tag>
                  </Show>
                </div>
                <Show when={s.args}>
                  <pre class="mono">{s.args}</pre>
                </Show>
              </div>
            )}
          </For>
        </div>
      </Show>
      <Show when={reply()}>
        <div class="reply">{reply()}</div>
      </Show>
    </section>
  );
}

function Event(props: { event: LogEvent; tz: string }) {
  return (
    <div class="log-event">
      <span class="sub mono">{clock(props.event.at, props.tz)}</span>
      <Tag>{EVENTS[props.event.kind] ?? props.event.kind}</Tag>
      <span class="detail-text">{props.event.detail}</span>
    </div>
  );
}
