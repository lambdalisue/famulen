import { createAsync, query } from "@solidjs/router";
import { For, Show } from "solid-js";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { hrefOf } from "~/lib/records";

const checkQuery = query(async () => (await api.notebook.checkNotebook({})).findings, "notebookCheck");

/** Where the notebook strays from its rules, and the fix for what has one. */
export default function NotebookCheck() {
  const findings = createAsync(() => checkQuery());
  const href = (path: string) => {
    const m = /^records\/([^/]+)\/([^/]+)\.md$/.exec(path);
    return m ? hrefOf(m[1], m[2]) : undefined;
  };
  return (
    <section class="pref" aria-labelledby="notebook-check">
      <div class="pref-head">
        <h2 id="notebook-check">手帳の書き方</h2>
        <p class="sub">
          手帳のファイルが決まりどおりに書かれているか。手で書き換えたものは、求めるまで直さない。「決まりどおりに直す」は、並びや日時の書き方のように、直し方が一つに決まるものだけを直す。
        </p>
      </div>
      <Show when={(findings() ?? []).length > 0} fallback={<p class="sub">{findings() ? "決まりから外れているものはない。" : "確かめている"}</p>}>
        <div class="list">
          <For each={findings()}>
            {(f) => (
              <div class="item">
                <span class={`tag${f.error ? " warn" : ""}`}>{f.error ? "誤り" : "注意"}</span>
                <span class="grow" style={{ "overflow-wrap": "anywhere", "font-size": "13px" }}>
                  {f.why}
                  <br />
                  <Show when={href(f.path)} fallback={<span class="sub mono">{f.path}:{f.line}</span>}>
                    {(h) => (
                      <a class="sub mono" href={h()}>
                        {f.path}:{f.line}
                      </a>
                    )}
                  </Show>
                </span>
              </div>
            )}
          </For>
        </div>
        <div>
          <button type="button" class="btn" onClick={() => void act(() => api.notebook.fixNotebook({}), "直した")}>
            決まりどおりに直す
          </button>
        </div>
      </Show>
    </section>
  );
}
