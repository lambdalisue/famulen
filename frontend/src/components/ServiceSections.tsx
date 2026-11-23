import { clock } from "~/lib/dates";
import Keyed from "~/components/Keyed";
import { Switch } from "@ark-ui/solid/switch";
import { A, createAsync, query } from "@solidjs/router";
import { For, type JSX, Show } from "solid-js";
import Icon from "~/components/Icon";
import LookoutEdit from "~/components/LookoutEdit";
import LookoutInterval from "~/components/LookoutInterval";
import type { Channel } from "~/gen/famulen/v1/services_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { calendarsQuery } from "~/lib/calendar";
import { channelsQuery, workspacesQuery } from "~/lib/connections";
import * as icons from "~/lib/icons";

const lookoutQuery = query(() => api.lookout.getLookout({}), "lookout");

/** A section of a settings page: a heading, a plain note under it, then what it controls. */
export function Pref(props: { title: string; note: JSX.Element; children?: JSX.Element; action?: JSX.Element }) {
  return (
    <section class="pref">
      <div class="pref-head">
        <div class="row">
          <h2 class="grow">{props.title}</h2>
          {props.action}
        </div>
        <p class="sub">{props.note}</p>
      </div>
      {props.children}
    </section>
  );
}

/** Whether the secretary takes what is said through one channel. */
export function ChannelSwitch(props: { channel: Channel; children: JSX.Element }) {
  return (
    <Switch.Root
      class="wg-row wg-switch"
      checked={props.channel.enabled}
      onCheckedChange={(d) =>
        void act(
          () =>
            api.channels.setChannelEnabled({
              connection: props.channel.connection,
              receiver: props.channel.receiver,
              enabled: d.checked,
            }),
          d.checked ? "ここで話しかけを受ける" : "ここでは話しかけを受けない",
        )
      }
    >
      <Switch.Label class="grow">{props.children}</Switch.Label>
      <Switch.Control class="switch">
        <Switch.Thumb class="switch-thumb" />
      </Switch.Control>
      <Switch.HiddenInput />
    </Switch.Root>
  );
}

/** Where approvals are asked, for a Slack workspace's channel. */
export function MainWorkspace(props: { connection: string }) {
  const workspaces = createAsync(() => workspacesQuery());
  const w = () => workspaces()?.workspaces.find((x) => x.id === props.connection);
  return (
    <Show when={w()}>
      {(ws) => (
        <Show
          when={!ws().main}
          fallback={<span class="tag aide">確認を受けるところ</span>}
        >
          <button
            type="button"
            class="btn small"
            onClick={() => act(() => api.channels.setMainWorkspace({ id: ws().id }), "確認をここで受ける")}
          >
            確認をここで受ける
          </button>
        </Show>
      )}
    </Show>
  );
}

/** The channels one connection gives: Slack's events, a device's voice. */
export function ChannelsSection(props: { connection: string }) {
  const channels = createAsync(() => channelsQuery());
  const mine = () => (channels() ?? []).filter((c) => c.connection === props.connection);
  return (
    <Show when={mine().length > 0}>
      <Pref
        title="チャネル"
        note={
          <>
            ここで秘書に話しかけられる。切ると、ここからの話しかけを受けなくなる。つながりは残るので、道具と見張りには使える。
            <A href="/settings/channels">チャネルの一覧</A>
          </>
        }
      >
        <For each={mine()}>
          {(c) => (
            <div class="row">
              <ChannelSwitch channel={c}>ここで話しかけを受ける</ChannelSwitch>
              <MainWorkspace connection={c.connection} />
            </div>
          )}
        </For>
      </Pref>
    </Show>
  );
}

/** The colors a calendar can take, as the workgroups use them. */
const COLORS: [string, string][] = [
  ["青", "#2a78d6"],
  ["橙", "#E0622F"],
  ["青緑", "#0F6E68"],
  ["紫", "#6250d6"],
  ["緑", "#008300"],
];

/** A Google account's calendars: added, removed and read again here. */
export function CalendarsSection(props: { connection: string; account: string; timezone: string }) {
  const list = createAsync(() => calendarsQuery());
  const mine = () => (list() ?? []).filter((c) => c.googleConnection === props.connection);
  const add = async (e: SubmitEvent) => {
    e.preventDefault();
    const form = e.currentTarget;
    if (!(form instanceof HTMLFormElement)) return;
    const f = new FormData(form);
    const done = await act(
      () =>
        api.calendar.addCalendar({
          name: String(f.get("name") ?? "").trim(),
          color: String(f.get("color") ?? ""),
          googleConnection: props.connection,
          googleCalendar: String(f.get("calendar_id") ?? "").trim(),
        }),
      "足した",
    );
    if (done) form.reset();
  };
  return (
    <Pref
      title="カレンダー"
      note="このアカウントのカレンダーを足すと、そのまま読める。Google 以外のカレンダーや祝日の一覧は、Google カレンダーで購読してから足す。ワークグループごとの見え方は、ワークグループの頁で決める。"
      action={
        <button
          type="button"
          class="btn small"
          aria-label="カレンダーを読み直す"
          onClick={() => act(() => api.calendar.refreshCalendars({}), "読み直している")}
        >
          <Icon svg={icons.refresh} size={14} />
        </button>
      }
    >
      <Show when={mine().length > 0} fallback={<p class="sub">まだない。メインのカレンダーは、ID を空のまま足す。</p>}>
        <div class="list">
          <For each={mine()}>
            {(c) => (
              <div class="item">
                <span
                  style={{
                    display: "inline-block",
                    width: "12px",
                    height: "12px",
                    "border-radius": "3px",
                    background: c.color,
                  }}
                />
                <div class="grow">
                  <b>{c.name}</b>
                  <br />
                  <span class="mono sub">{c.googleCalendar === "primary" ? "メインのカレンダー" : c.googleCalendar}</span>
                </div>
                <span class="sub">
                  <Show
                    when={c.error}
                    fallback={c.fetchedAt > 0n ? `${clock(c.fetchedAt, props.timezone)} に読んだ` : "まだ読んでいない"}
                  >
                    <span class="tag warn">読めない</span> {c.error}
                  </Show>
                </span>
                <button
                  type="button"
                  class="btn small danger"
                  aria-label={`${c.name}を外す`}
                  onClick={() => act(() => api.calendar.removeCalendar({ id: c.id }), "外した")}
                >
                  <Icon svg={icons.trash} size={12} />
                </button>
              </div>
            )}
          </For>
        </div>
      </Show>
      <form class="row" onSubmit={add}>
        <input class="input" name="name" required value={props.account} aria-label="カレンダーの名前" />
        <input
          class="input grow mono"
          name="calendar_id"
          placeholder="カレンダー ID（空ならメインのカレンダー）"
          aria-label="カレンダー ID"
        />
        <select class="input" name="color" aria-label="色">
          <For each={COLORS}>{([l, c]) => <option value={c}>{l}</option>}</For>
        </select>
        <button class="btn primary">足す</button>
      </form>
    </Pref>
  );
}

/** What the lookout reads from one connection, each turned on or off with how often it looks. */
export function LookoutSection(props: { connection: string }) {
  const w = createAsync(() => lookoutQuery());
  const mine = () =>
    (w()?.sources ?? []).filter(
      (s) => s.connection === props.connection || s.connection === `slack:${props.connection}`,
    );
  return (
    <Pref
      title="見張り"
      note={
        <>
          このアカウントに届くもの（受信箱やメンションなど）を見張るか、どれほどの間隔でまとめて見るか。間隔は既定に従うか、この見張りだけ上書きする。既定の間隔と何を拾うかは
          <A href="/settings/lookout">見張りの頁</A>で決める。
        </>
      }
    >
      <Show when={mine().length > 0} fallback={<p class="sub">このアカウントを見張るものはまだない。</p>}>
        <Keyed each={mine()} by={(s) => s.id}>
          {(s) => (
            <div class="row">
              <Switch.Root
                class="wg-row wg-switch grow"
                checked={s().enabled}
                onCheckedChange={(d) =>
                  void act(
                    () => api.lookout.setLookoutEnabled({ id: s().id, enabled: d.checked }),
                    d.checked ? "見張りを始めた" : "止めた",
                  )
                }
              >
                <Switch.Label class="grow">{s().name}</Switch.Label>
                <Switch.Control class="switch">
                  <Switch.Thumb class="switch-thumb" />
                </Switch.Control>
                <Switch.HiddenInput />
              </Switch.Root>
              <LookoutInterval source={s()} defaults={w()?.defaults ?? []} place="connection" label={`${s().name}をまとめて見る間隔（分）`} />
              <LookoutEdit source={s()} defaults={w()?.defaults ?? []} sources={w()?.sources ?? []} />
            </div>
          )}
        </Keyed>
      </Show>
    </Pref>
  );
}
