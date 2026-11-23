import { A, createAsync } from "@solidjs/router";
import { createSignal, For, onMount, Show } from "solid-js";
import Icon from "~/components/Icon";
import { ChannelSwitch, MainWorkspace } from "~/components/ServiceSections";
import { channelsQuery } from "~/lib/connections";
import { personaQuery } from "~/lib/data";
import * as icons from "~/lib/icons";
import { setSelection, setView } from "~/lib/view";

/**
 * Every place the owner can talk with the secretary: this console, and what
 * the connected services give, each turned on or off.
 */
export default function Channels() {
  setView("設定 · チャネル");
  const channels = createAsync(() => channelsQuery());
  const persona = createAsync(() => personaQuery());
  // The origin is only known in the browser; rendering it on the server too
  // would not match what hydration sees.
  const [origin, setOrigin] = createSignal("");
  onMount(() => setOrigin(location.origin));

  return (
    <>
      <h1>チャネル</h1>
      <p class="sub">
        {persona()?.name ?? "秘書"} と話せるところ。このブラウザはいつでも使える。Slack などは
        <A href="/settings/services">サービス</A>
        で足すとここに並び、切るとそこからの話しかけを受けなくなる。切っても、サービスのつながりは残る。
      </p>
      <div class="list">
        <div class="item">
          <Icon svg={icons.deviceLaptop} size={18} />
          <span class="grow">
            <b>このブラウザ</b>
            <br />
            <span class="sub">コンソール{origin() ? ` · ${origin()}` : ""}</span>
          </span>
          <span class="tag confirmed">使える</span>
        </div>
        <For each={channels() ?? []}>
          {(c) => (
            <div class="item" onClick={() => setSelection(`${c.service} のチャネル ${c.name}`)}>
              <Icon svg={c.definition === "slack" ? icons.brandSlack : icons.devices} size={18} />
              <A
                class="grow"
                href={`/settings/services?id=${encodeURIComponent(c.connection)}`}
                style={{ color: "inherit", "text-decoration": "none" }}
              >
                <b>{c.name}</b>
                <br />
                <span class="sub">{c.service}</span>
              </A>
              <span class={`tag${c.status === "つながっている" ? " confirmed" : " warn"}`}>{c.status}</span>
              <MainWorkspace connection={c.connection} />
              <ChannelSwitch channel={c}>
                <span class="sub">{c.enabled ? "受ける" : "受けない"}</span>
              </ChannelSwitch>
            </div>
          )}
        </For>
      </div>
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
