import { A, createAsync, useSearchParams } from "@solidjs/router";
import { createEffect, For, type JSX, Match, Show, Switch } from "solid-js";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { personaQuery } from "~/lib/data";
import { memoryFileQuery, stamp } from "~/lib/memory";
import { setView } from "~/lib/view";

const MODES: [string, string][] = [
  ["read", "読む"],
  ["write", "書く"],
  ["history", "経緯"],
];

const ATTRS = /\s+\{([\w-]+=[^}]*)\}\s*$/;

/** A line's text, with the item attributes at its end set apart. */
function Line(props: { text: string }) {
  const m = () => ATTRS.exec(props.text);
  return (
    <>
      {m() ? props.text.slice(0, m()?.index) : props.text}
      <Show when={m()}>{(a) => <span class="attrs mono">{a()[1]}</span>}</Show>
    </>
  );
}

/**
 * The Markdown a memory file is written in: headings, lists, fenced code and
 * paragraphs. Built as elements rather than an HTML string, so nothing in the
 * file can inject markup.
 */
function Markdown(props: { text: string }): JSX.Element {
  const blocks = () => {
    const out: JSX.Element[] = [];
    let list: string[] = [];
    let para: string[] = [];
    let code: string[] | undefined;
    const flush = () => {
      if (list.length > 0) {
        const items = list;
        out.push(
          <ul>
            <For each={items}>
              {(t) => (
                <li>
                  <Line text={t} />
                </li>
              )}
            </For>
          </ul>,
        );
        list = [];
      }
      if (para.length > 0) {
        const text = para.join(" ");
        out.push(
          <p>
            <Line text={text} />
          </p>,
        );
        para = [];
      }
    };
    for (const line of props.text.split("\n")) {
      if (code !== undefined) {
        if (line.startsWith("```")) {
          const body = code.join("\n");
          out.push(<pre>{body}</pre>);
          code = undefined;
        } else code.push(line);
        continue;
      }
      if (line.startsWith("```")) {
        flush();
        code = [];
        continue;
      }
      const heading = /^(#{1,6})\s+(.*)$/.exec(line);
      const item = /^\s*[-*]\s+(.*)$/.exec(line);
      if (heading) {
        flush();
        const text = heading[2];
        const level = heading[1].length;
        out.push(level === 1 ? <h1>{text}</h1> : level === 2 ? <h2>{text}</h2> : <h3>{text}</h3>);
      } else if (item) {
        if (para.length > 0) flush();
        list.push(item[1]);
      } else if (line.trim() === "") {
        flush();
      } else {
        if (list.length > 0) flush();
        para.push(line.trim());
      }
    }
    if (code !== undefined) out.push(<pre>{code.join("\n")}</pre>);
    flush();
    return out;
  };
  return <>{blocks()}</>;
}

export default function MemoryFile() {
  const [params] = useSearchParams();
  const path = () => (typeof params.path === "string" ? params.path : "");
  const mode = () => (typeof params.mode === "string" ? params.mode : "read");
  const file = createAsync(async () => (path() ? memoryFileQuery(path()) : undefined));
  const persona = createAsync(() => personaQuery());
  const aide = () => persona()?.name || "秘書";
  const tz = () => persona()?.timezone ?? "UTC";
  createEffect(() => setView(`記憶 · ${path()}`));

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
          <Switch fallback={<article class="card markdown"><Markdown text={f().text} /></article>}>
            <Match when={mode() === "write"}>
              <div style={{ display: "flex", "flex-direction": "column", gap: "8px" }}>
                <label for="mem-edit" class="sr">
                  {path()} を書く
                </label>
                <textarea
                  id="mem-edit"
                  class="input mono"
                  style={{ "min-height": "420px", "font-size": "13px", "line-height": 1.8 }}
                  value={f().text}
                  onChange={(e) => save(e.currentTarget.value)}
                >
                  {/* Also the content: the server writes `value` as an attribute, which a textarea ignores, and hydration does not set it. */}
                  {f().text}
                </textarea>
                <span class="sub">書き終えて離れると保存し、あなたが直したこととして経緯に残る</span>
              </div>
            </Match>
            <Match when={mode() === "history"}>
              <div class="list">
                <For each={f().history} fallback={<p class="empty">まだ経緯はない</p>}>
                  {(c) => {
                    const mine = c.author === "Famulen";
                    return (
                      <div class="item">
                        <span class="mono sub" style={{ width: "120px" }}>
                          {stamp(c.at, tz())}
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
