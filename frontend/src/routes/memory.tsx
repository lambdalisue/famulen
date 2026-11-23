import { A, createAsync, useLocation, useSearchParams } from "@solidjs/router";
import { For, type JSX } from "solid-js";
import { layerHref, layerOfFile, layersQuery } from "~/lib/memory";
import { useOwner } from "~/lib/owner";
import "./memory.css";

/** The memory pages share one frame: the layers beside the chosen page. */
export default function Memory(props: { children?: JSX.Element }) {
  const location = useLocation();
  const [params] = useSearchParams();
  const layers = createAsync(() => layersQuery());
  const owner = useOwner();
  const current = () => {
    const rest = location.pathname.replace(/^\/memory\/?/, "");
    if (rest === "") return "dream";
    if (rest === "file") {
      const path = params.path;
      return layerOfFile(typeof path === "string" ? path : "");
    }
    return rest;
  };
  return (
    <div class="pane split" style={{ margin: "-22px -26px", flex: 1 }}>
      <nav class="side" aria-label="記憶の層" style={{ width: "230px" }}>
        <For each={layers()}>
          {(l) => (
            <A
              class={`nav-item${l.id === current() ? " on" : ""}`}
              href={layerHref(l.id)}
              end
              style={{ "flex-direction": "column", "align-items": "flex-start", gap: "2px" }}
            >
              <span class="row" style={{ width: "100%" }}>
                <b style={{ "font-size": "14px" }} class="grow">
                  {l.label}
                </b>
                <span class="mono sub">{l.count}</span>
              </span>
              <span class="sub">{l.help}</span>
            </A>
          )}
        </For>
        <p class="sub" style={{ "margin-top": "auto" }}>
          記憶は {owner.aide()}{" "}
          がやりとりから覚えたこと。予定・タスク・相手・物・契約は記録なので、ここには入らない。変更はすべて Git に残る。
        </p>
      </nav>
      <div class="detail">{props.children}</div>
    </div>
  );
}
