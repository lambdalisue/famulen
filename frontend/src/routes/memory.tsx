import { createAsync, useLocation } from "@solidjs/router";
import { For, type JSX, Show } from "solid-js";
import { layerHref, layerOfFile, layersQuery } from "~/lib/memory";
import { useOwner } from "~/lib/owner";
import { useParam } from "~/lib/params";

/** What memory is, as it is said on each list of its layers. */
export function AboutMemory() {
  const owner = useOwner();
  return (
    <p class="sub">
      記憶は {owner.aide()} がやりとりから覚えたこと。予定・タスク・相手・物・契約は記録なので、ここには入らない。変更はすべて Git に残る。
    </p>
  );
}

/** The memory pages share one frame: the layers beside the chosen page. */
export default function Memory(props: { children?: JSX.Element }) {
  const location = useLocation();
  const path = useParam("path");
  const layers = createAsync(() => layersQuery());
  const current = () => {
    const rest = location.pathname.replace(/^\/memory\/?/, "");
    if (rest === "") return "dream";
    return rest === "file" ? layerOfFile(path()) : rest;
  };
  return (
    <div class="framed">
      <nav class="side" aria-label="記憶の層">
        <For each={layers()}>
          {(l) => (
            <a
              class="nav-item"
              classList={{ on: l.id === current() }}
              href={layerHref(l.id)}
              aria-current={l.id === current() ? "page" : undefined}
            >
              <span class="stack">
                <span class="row">
                  <span class="grow">{l.label}</span>
                  <Show when={l.count}>
                    <span class="mono sub">{l.count}</span>
                  </Show>
                </span>
                <span class="sub">{l.help}</span>
              </span>
            </a>
          )}
        </For>
        <AboutMemory />
      </nav>
      <div class="detail">
        <div class="page">{props.children}</div>
      </div>
    </div>
  );
}
