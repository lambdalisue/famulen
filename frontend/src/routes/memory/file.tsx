import { dateTimeText } from "~/lib/dates";
import { A, createAsync, useSearchParams } from "@solidjs/router";
import { createEffect, For, Match, Show, Switch, untrack } from "solid-js";
import Markdown from "~/components/Markdown";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { createDraft } from "~/lib/draft";
import { memoryFileQuery } from "~/lib/memory";
import { useOwner } from "~/lib/owner";
import { oneParam } from "~/lib/params";
import { setView } from "~/lib/view";

const MODES: [string, string][] = [
  ["read", "読む"],
  ["write", "書く"],
  ["history", "経緯"],
];

/** A file as it is being written: what is typed stays when the page's data loads again. */
function Writing(props: { path: string; text: string; onSave: (text: string) => void }) {
  const [draft, setDraft] = createDraft(() => props.text);
  return (
    <div style={{ display: "flex", "flex-direction": "column", gap: "8px" }}>
      <label for="mem-edit" class="sr">
        {props.path} を書く
      </label>
      <textarea
        id="mem-edit"
        class="input mono"
        style={{ "min-height": "420px", "font-size": "13px", "line-height": 1.8 }}
        value={draft()}
        onInput={(e) => setDraft(e.currentTarget.value)}
        onChange={(e) => props.onSave(e.currentTarget.value)}
      >
        {/* Also the content: the server writes `value` as an attribute, which a textarea ignores, and hydration does not set it. */}
        {untrack(draft)}
      </textarea>
      <span class="sub">書き終えて離れると保存し、あなたが直したこととして経緯に残る</span>
    </div>
  );
}

export default function MemoryFile() {
  const [params] = useSearchParams();
  const path = () => oneParam(params.path);
  const mode = () => oneParam(params.mode) || "read";
  const file = createAsync(async () => (path() ? memoryFileQuery(path()) : undefined));
  const owner = useOwner();
  const aide = owner.aide;
  const tz = owner.tz;
  createEffect(() => setView("記憶", path()));

  const save = async (text: string) => {
    const content = text.replace(/\r\n/g, "\n");
    if (content === file()?.text) return;
    await act(() => api.memory.writeFile({ path: path(), text: content }), "保存した");
  };

  return (
    <Show when={path()} fallback={<p class="empty">ファイルを選んでいない</p>}>
      <div class="row">
        <span class="mono sub">{path()}</span>
        <div class="seg" style={{ "margin-left": "auto" }}>
          <For each={MODES}>
            {([m, l]) => (
              <A
                class={m === mode() ? "on" : ""}
                href={`/memory/file?path=${encodeURIComponent(path())}&mode=${m}`}
              >
                {l}
              </A>
            )}
          </For>
        </div>
      </div>
      <Show when={file()}>
        {(f) => (
          <Switch fallback={<div class="card"><Markdown text={f().text} attributes /></div>}>
            <Match when={mode() === "write"}>
              <Writing path={path()} text={f().text} onSave={(text) => void save(text)} />
            </Match>
            <Match when={mode() === "history"}>
              <div class="list">
                <For each={f().history} fallback={<p class="empty">まだ経緯はない</p>}>
                  {(c) => {
                    const mine = c.author === "Famulen";
                    return (
                      <div class="item">
                        <span class="mono sub" style={{ width: "120px" }}>
                          {dateTimeText(c.at, tz())}
                        </span>
                        <span class={`tag ${mine ? "aide" : "said"}`}>{mine ? aide() : "あなた"}</span>
                        <span class="grow" style={{ "font-size": "13px", "white-space": "pre-wrap" }}>
                          {c.message}
                        </span>
                      </div>
                    );
                  }}
                </For>
              </div>
            </Match>
          </Switch>
        )}
      </Show>
    </Show>
  );
}
