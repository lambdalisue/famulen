import { For, Show } from "solid-js";
import { IndexRow } from "~/components/IndexRow";
import { List } from "~/components/ui";
import { setView } from "~/lib/view";
import { NAV } from "../settings";

/** The narrow layout's index of the settings pages. */
export default function SettingsMenu() {
  setView("設定");
  return (
    <>
      <p class="sub">設定も会話から変えられる。この画面は、いまどうなっているかを確かめる場所。</p>
      <For each={NAV}>
        {([group, items]) => (
          <section class="stack">
            <Show when={group}>
              <h3>{group}</h3>
            </Show>
            <List>
              <For each={items}>{([href, label, icon]) => <IndexRow href={href} icon={icon} name={label} />}</For>
            </List>
          </section>
        )}
      </For>
    </>
  );
}
