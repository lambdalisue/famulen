import { A } from "@solidjs/router";
import { For, Show } from "solid-js";
import * as icons from "~/lib/icons";
import { setView } from "~/lib/view";
import { NAV } from "../settings";
import { Icon, List } from "~/components/ui";

/** The narrow layout's index of the settings pages. */
export default function SettingsMenu() {
  setView("設定");
  return (
    <>
      <p class="sub">設定も会話から変えられる。この画面は、いまどうなっているかを確かめる場所。</p>
      <For each={NAV}>
        {([group, items]) => (
          <>
            <Show when={group}>
              <h3>{group}</h3>
            </Show>
            <List>
              <For each={items}>
                {([href, label, icon]) => (
                  <A class="item index-row" href={href}>
                    <span class="index-icon">
                      <Icon svg={icon} size={20} />
                    </span>
                    <b class="grow">{label}</b>
                    <Icon svg={icons.chevronRight} size={18} />
                  </A>
                )}
              </For>
            </List>
          </>
        )}
      </For>
    </>
  );
}
