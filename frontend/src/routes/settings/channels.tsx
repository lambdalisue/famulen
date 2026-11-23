import { A, createAsync } from "@solidjs/router";
import { createSignal, For, onMount, Show } from "solid-js";
import { ChannelSwitch, MainWorkspace } from "~/components/ServiceSections";
import { ConnectionState } from "~/gen/famulen/v1/services_pb";
import { channelsQuery } from "~/lib/connections";
import * as icons from "~/lib/icons";
import { useOwner } from "~/lib/owner";
import { List, ListRow, PageHeader, Tag } from "~/components/ui";
import { setView } from "~/lib/view";

/**
 * Every place the owner can talk with the secretary: this console, and what
 * the connected services give, each turned on or off.
 */
export default function Channels() {
  setView("設定", "チャネル");
  const channels = createAsync(() => channelsQuery());
  const owner = useOwner();
  // The origin is only known in the browser; rendering it on the server too
  // would not match what hydration sees.
  const [origin, setOrigin] = createSignal("");
  onMount(() => setOrigin(location.origin));

  return (
    <>
      <PageHeader title="チャネル" note={<>
        {owner.aide()} と話せるところ。このブラウザはいつでも使える。Slack などは
        <A href="/settings/services">サービス</A>
        で足すとここに並び、切るとそこからの話しかけを受けなくなる。切っても、サービスのつながりは残る。
      </>} />
      <List>
        <ListRow icon={icons.deviceLaptop} title="このブラウザ" sub={`コンソール${origin() ? ` · ${origin()}` : ""}`} end={<Tag tone="ok">使える</Tag>} />
        <For each={channels() ?? []}>
          {(c) => (
            <ListRow
              icon={c.definition === "slack" ? icons.brandSlack : icons.devices}
              href={`/settings/services?id=${encodeURIComponent(c.connection)}`}
              title={c.name}
              sub={c.service}
              end={
                <>
                  <Tag tone={c.state === ConnectionState.CONNECTED ? "ok" : "warn"} title={c.status}>
                    {c.status}
                  </Tag>
                  <MainWorkspace connection={c.connection} />
                  <ChannelSwitch channel={c}>
                    <span class="sub">{c.enabled ? "受ける" : "受けない"}</span>
                  </ChannelSwitch>
                </>
              }
            />
          )}
        </For>
      </List>
      <Show when={channels() && channels()?.length === 0}>
        <p class="sub">
          Slack のワークスペースや眼鏡などは、<A href="/settings/services">サービス</A>で足すとここに並ぶ。
        </p>
      </Show>
      <p class="sub">
        メンションのあと、そのチャンネルを注視する長さは<A href="/settings/roles/triage">トリアージ</A>で決める。
      </p>
    </>
  );
}
