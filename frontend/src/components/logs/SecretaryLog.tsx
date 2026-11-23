import { createAsync, query } from "@solidjs/router";
import { createMemo, createSignal, For, Show } from "solid-js";
import Icon from "~/components/Icon";
import type { LogEvent, LogItem, LogTurn } from "~/gen/famulen/v1/settings_pb";
import { api } from "~/lib/api";
import { personaQuery } from "~/lib/data";
import * as icons from "~/lib/icons";
import { setSelection } from "~/lib/view";
import "./log.css";
import { createPages, PAGE, PageNav } from "./Pages";

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

function dayOf(at: bigint, tz: string): string {
  return new Intl.DateTimeFormat("ja-JP", {
    timeZone: tz || "UTC",
    month: "long",
    day: "numeric",
    weekday: "short",
  }).format(new Date(Number(at) * 1000));
}

function clockOf(at: bigint, tz: string): string {
  return new Intl.DateTimeFormat("ja-JP", {
    timeZone: tz || "UTC",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  }).format(new Date(Number(at) * 1000));
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
  const persona = createAsync(() => personaQuery());
  const tz = () => persona()?.timezone ?? "";

  const days = createMemo(() => {
    const groups: { day: string; items: LogItem[] }[] = [];
    for (const item of items()) {
      const day = dayOf(atOf(item), tz());
      const last = groups.at(-1);
      if (last?.day === day) last.items.push(item);
      else groups.push({ day, items: [item] });
    }
    return groups;
  });

  return (
    <>
      <div class="row" style={{ gap: "6px" }}>
        <For each={FILTERS}>
          {([key, label]) => (
            <button
              type="button"
              class={`btn small${show() === key ? " primary" : ""}`}
              aria-pressed={show() === key}
              onClick={() => choose(key)}
            >
              {label}
            </button>
          )}
        </For>
      </div>
      <div style={{ display: "flex", "flex-direction": "column", gap: "8px" }}>
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
          <p class="empty">まだない</p>
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
    if (next) setSelection([...`ログ · ${clockOf(t().at, props.tz)} ${t().asked || reply()}`].slice(0, 120).join(""));
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
      <div class="row" style={{ gap: "8px" }}>
        <span class="sub mono">{clockOf(t().at, props.tz)}</span>
        <Show when={t().origin}>
          <span class="tag">{t().origin}から</span>
        </Show>
        <Show when={t().steps.length > 0}>
          <span class="tag">
            道具 {t().steps.length} 回
          </span>
        </Show>
        <For each={stopped()}>
          {(s) => (
            <span class="tag warn">
              {s.tool} · {s.outcome}
            </span>
          )}
        </For>
        <span class="sub" style={{ "margin-left": "auto" }}>
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
              <span class="tag">
                <Icon svg={icons.plugConnected} size={12} />
                {s.service ? `${s.service} · ` : ""}
                {s.tool}
              </span>
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
                    <span class="tag warn">{s.outcome}</span>
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
      <span class="sub mono">{clockOf(props.event.at, props.tz)}</span>
      <span class="tag">{EVENTS[props.event.kind] ?? props.event.kind}</span>
      <span class="detail-text">{props.event.detail}</span>
    </div>
  );
}
