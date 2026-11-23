import { A, createAsync } from "@solidjs/router";
import { For, Show } from "solid-js";
import Icon from "~/components/Icon";
import * as icons from "~/lib/icons";
import { layerHref, layersQuery } from "~/lib/memory";
import { useOwner } from "~/lib/owner";
import { setView } from "~/lib/view";

/** The memory layers as a list of pages, for the narrow layout. */
export default function MemoryMenu() {
  setView("記憶");
  const layers = createAsync(() => layersQuery());
  const owner = useOwner();
  return (
    <>
      <div class="list">
        <For each={layers()}>
          {(l) => (
            <A class="item index-row" href={layerHref(l.id)}>
              <span class="index-icon">
                <Icon svg={icons.brain} size={20} />
              </span>
              <span class="grow">
                <b style={{ "font-size": "16px" }}>{l.label}</b>
                <br />
                <span class="sub">{l.help}</span>
              </span>
              <Show when={l.count}>
                <span class="mono sub">{l.count}</span>
              </Show>
              <Icon svg={icons.chevronRight} size={18} />
            </A>
          )}
        </For>
      </div>
      <p class="sub">
        記憶は {owner.aide()}{" "}
        がやりとりから覚えたこと。予定・タスク・相手・物・契約は記録なので、ここには入らない。変更はすべて Git に残る。
      </p>
    </>
  );
}
