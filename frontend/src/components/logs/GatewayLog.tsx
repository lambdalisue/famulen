import { createAsync, query } from "@solidjs/router";
import { createSignal, For, Show } from "solid-js";
import type { GatewayLogEntry } from "~/gen/famulen/v1/vault_pb";
import { api } from "~/lib/api";
import { connectionsQuery } from "~/lib/connections";
import { personaQuery } from "~/lib/data";
import { shortTime } from "~/lib/time";
import "./log.css";
import { createPages, PAGE, PageNav } from "./Pages";

type Filter = { actor: string; connection: string; decision: string };

const gatewayLogQuery = query(async (filter: Filter, before: number) => {
  const page = await api.gatewayLog.listGatewayLog({ ...filter, before: BigInt(before), limit: PAGE });
  return { rows: page.entries, more: page.more };
}, "gatewayLog");

const ACTORS: [string, string][] = [
  ["", "誰でも"],
  ["owner", "主人"],
  ["secretary", "秘書"],
  ["worker", "手代"],
  ["lookout", "見張り"],
  ["assessor", "見極め"],
];

const DECISIONS: [string, string][] = [
  ["", "すべて"],
  ["allow", "通した"],
  ["confirm", "確認"],
  ["deny", "断った"],
];

/** How a decision reads on a row. */
const DECIDED: Record<string, [string, string]> = {
  allow: ["通した", ""],
  confirm: ["確認", "warn"],
  deny: ["断った", "danger"],
};

function Entry(props: { e: GatewayLogEntry; tz: string; names: Map<string, string> }) {
  const actor = () => ACTORS.find(([k]) => k === props.e.actor)?.[1] ?? props.e.actor;
  const decided = () => DECIDED[props.e.decision] ?? [props.e.decision, ""];
  const where = () => props.names.get(props.e.connection) ?? props.e.connection;
  return (
    <div class="item gateway-log-entry" style={{ "flex-wrap": "wrap", gap: "6px" }}>
      <span class="sub mono">{shortTime(props.e.at, props.tz)}</span>
      <span class={`tag ${decided()[1]}`}>{decided()[0]}</span>
      <span class="tag">{actor()}</span>
      <b class="mono">
        {where() ? `${where()} · ` : ""}
        {props.e.tool}
      </b>
      <Show when={props.e.outcome}>
        <span class="sub">{props.e.outcome}</span>
      </Show>
      <Show when={props.e.ms > 0n}>
        <span class="sub">{props.e.ms.toString()} ms</span>
      </Show>
      <Show when={props.e.reason || props.e.secret || props.e.digest}>
        <div class="sub" style={{ width: "100%", "overflow-wrap": "anywhere" }}>
          {props.e.reason}
          <Show when={props.e.secret}>
            {props.e.reason ? " · " : ""}秘密 <span class="mono">{props.e.secret}</span>
          </Show>
          <Show when={props.e.digest}>
            {" · "}
            <span class="mono">{props.e.digest}</span>
          </Show>
        </div>
      </Show>
    </div>
  );
}

/**
 * What the gateway let through, asked about or refused, and what changed in
 * it. With `actor`, only that caller's rows are read and it cannot be changed.
 */
export default function GatewayLog(props: { actor?: string }) {
  const [filter, setFilter] = createSignal<Filter>({ actor: props.actor ?? "", connection: "", decision: "" });
  const pages = createPages((before) => gatewayLogQuery(filter(), before));
  const persona = createAsync(() => personaQuery());
  const tz = () => persona()?.timezone ?? "UTC";
  const connections = createAsync(() => connectionsQuery());
  const names = () => new Map((connections()?.connections ?? []).map((c) => [c.id, c.name]));
  const choose = (key: keyof Filter, value: string) => {
    setFilter((f) => ({ ...f, [key]: value }));
    pages.reset();
  };

  return (
    <>
      <p class="sub">
        ゲートウェイが通したもの・確認に回したもの・断ったものと、つなぎ先・権限・金庫・宛先の変更。本文と秘密の中身は残らない。
      </p>
      <div class="row" style={{ gap: "6px", "flex-wrap": "wrap" }}>
        <For each={DECISIONS}>
          {([key, label]) => (
            <button
              type="button"
              class={`btn small${filter().decision === key ? " primary" : ""}`}
              aria-pressed={filter().decision === key}
              onClick={() => choose("decision", key)}
            >
              {label}
            </button>
          )}
        </For>
        <Show when={!props.actor}>
          <select
            class="input"
            aria-label="呼ぶ者"
            value={filter().actor}
            onChange={(e) => choose("actor", e.currentTarget.value)}
          >
            <For each={ACTORS}>{([key, label]) => <option value={key}>{label}</option>}</For>
          </select>
        </Show>
        <select
          class="input"
          aria-label="つなぎ先"
          value={filter().connection}
          onChange={(e) => choose("connection", e.currentTarget.value)}
        >
          <option value="">どのつなぎ先も</option>
          <option value="vault">金庫</option>
          <For each={connections()?.connections ?? []}>{(c) => <option value={c.id}>{c.name}</option>}</For>
        </select>
      </div>
      <div class="list">
        <For each={pages.rows()}>{(e) => <Entry e={e} tz={tz()} names={names()} />}</For>
        <Show when={pages.loaded() && pages.rows().length === 0}>
          <p class="empty">まだない</p>
        </Show>
      </div>
      <PageNav pages={pages} />
    </>
  );
}
