import { own } from "~/lib/colors";
import { clock } from "~/lib/dates";
import { createAsync, query } from "@solidjs/router";
import { For, type JSX, Show } from "solid-js";
import LookoutEdit from "~/components/LookoutEdit";
import LookoutInterval from "~/components/LookoutInterval";
import type { Channel } from "~/gen/famulen/v1/services_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { calendarsQuery } from "~/lib/calendar";
import { channelsQuery, workspacesQuery } from "~/lib/connections";
import * as icons from "~/lib/icons";
import {
  Button,
  EmptyState,
  Form,
  IconButton,
  Keyed,
  List,
  ListRow,
  Section,
  Select,
  Switch,
  Tag,
  TextField,
  whenConfirmed,
} from "~/components/ui";
import { COLORS } from "~/lib/workgroup";

const lookoutQuery = query(() => api.lookout.getLookout({}), "lookout");

/** Whether the secretary takes what is said through one channel. */
export function ChannelSwitch(props: { channel: Channel; children: JSX.Element }) {
  return (
    <Switch
      end
      class="wg-switch"
      checked={props.channel.enabled}
      onChange={(enabled) =>
        void act(
          () => api.channels.setChannelEnabled({ connection: props.channel.connection, receiver: props.channel.receiver, enabled }),
          enabled ? "ここで話しかけを受ける" : "ここでは話しかけを受けない",
        )
      }
    >
      {props.children}
    </Switch>
  );
}

/** Where approvals are asked, for a Slack workspace's channel. */
export function MainWorkspace(props: { connection: string }) {
  const workspaces = createAsync(() => workspacesQuery());
  const w = () => workspaces()?.workspaces.find((x) => x.id === props.connection);
  return (
    <Show when={w()}>
      {(ws) => (
        <Show when={!ws().main} fallback={<Tag tone="aide">確認を受けるところ</Tag>}>
          <Button small onClick={() => act(() => api.channels.setMainWorkspace({ id: ws().id }), "確認をここで受ける")}>
            確認をここで受ける
          </Button>
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
      <Section
        title="チャネル"
        note={
          <>
            ここで秘書に話しかけられる。切ると、ここからの話しかけを受けなくなる。つながりは残るので、道具と見張りには使える。
            <a href="/settings/channels">チャネルの一覧</a>
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
      </Section>
    </Show>
  );
}

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
    <Section
      title="カレンダー"
      note="このアカウントのカレンダーを足すと、そのまま読める。Google 以外のカレンダーや祝日の一覧は、Google カレンダーで購読してから足す。ワークグループごとの見え方は、ワークグループの頁で決める。"
      action={
        <IconButton
          icon={icons.refresh}
          label="カレンダーを読み直す"
          onClick={() => act(() => api.calendar.refreshCalendars({}), "読み直している")}
        />
      }
    >
      <Show when={mine().length > 0} fallback={<EmptyState>まだない。メインのカレンダーは、ID を空のまま足す</EmptyState>}>
        <List>
          <For each={mine()}>
            {(c) => (
              <ListRow
                lead={<span class="swatch-square" style={{ background: own(c.color) }} />}
                title={c.name}
                sub={
                  <>
                    <span class="mono">{c.googleCalendar === "primary" ? "メインのカレンダー" : c.googleCalendar}</span>
                    <Show
                      when={c.error}
                      fallback={<span>{c.fetchedAt > 0n ? `${clock(c.fetchedAt, props.timezone)} に読んだ` : "まだ読んでいない"}</span>}
                    >
                      <Tag tone="warn">読めない</Tag>
                      <span>{c.error}</span>
                    </Show>
                  </>
                }
                end={
                  <IconButton
                    variant="danger"
                    icon={icons.trash}
                    label={`${c.name}を外す`}
                    onClick={() =>
                      whenConfirmed(
                        `カレンダー「${c.name}」を外す`,
                        "外す",
                        () => act(() => api.calendar.removeCalendar({ id: c.id }), "外した"),
                        "Google のカレンダーはそのまま残る。",
                      )
                    }
                  />
                }
              />
            )}
          </For>
        </List>
      </Show>
      <Form class="row" onSubmit={add}>
        <TextField name="name" required value={props.account} aria-label="カレンダーの名前" />
        <TextField
          class="grow mono"
          name="calendar_id"
          placeholder="カレンダー ID（空ならメインのカレンダー）"
          aria-label="カレンダー ID"
        />
        <Select name="color" aria-label="色">
          <For each={COLORS}>{([l, c]) => <option value={c}>{l}</option>}</For>
        </Select>
        <Button variant="primary" type="submit">
          足す
        </Button>
      </Form>
    </Section>
  );
}

/** What the lookout reads from one connection, each turned on or off with how often it looks. */
export function LookoutSection(props: { connection: string }) {
  const w = createAsync(() => lookoutQuery());
  const mine = () => (w()?.sources ?? []).filter((s) => s.connection === props.connection || s.connection === `slack:${props.connection}`);
  return (
    <Section
      title="見張り"
      note={
        <>
          このアカウントに届くもの（受信箱やメンションなど）を見張るか、どれほどの間隔でまとめて見るか。間隔は既定に従うか、この見張りだけ上書きする。既定の間隔と何を拾うかは
          <a href="/settings/lookout">見張りの頁</a>で決める。
        </>
      }
    >
      <Show when={mine().length > 0} fallback={<EmptyState>このアカウントを見張るものはまだない</EmptyState>}>
        <Keyed each={mine()} by={(s) => s.id}>
          {(s) => (
            <div class="row">
              <Switch
                class="wg-switch grow"
                checked={s().enabled}
                onChange={(enabled) =>
                  void act(() => api.lookout.setLookoutEnabled({ id: s().id, enabled }), enabled ? "見張りを始めた" : "止めた")
                }
              >
                {s().name}
              </Switch>
              <LookoutInterval
                source={s()}
                defaults={w()?.defaults ?? []}
                place="connection"
                label={`${s().name}をまとめて見る間隔（分）`}
              />
              <LookoutEdit source={s()} defaults={w()?.defaults ?? []} sources={w()?.sources ?? []} />
            </div>
          )}
        </Keyed>
      </Show>
    </Section>
  );
}
