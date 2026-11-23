import { createAsync, useBeforeLeave } from "@solidjs/router";
import { createEffect, createSignal, For, Match, on, Show, Switch } from "solid-js";
import Markdown from "~/components/Markdown";
import { Button, Callout, confirmed, EmptyState, List, ListRow, Loading, PageHeader, SegmentedLinks, Tag, TextArea } from "~/components/ui";
import type { FileContent } from "~/gen/famulen/v1/memory_pb";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { dateTimeText } from "~/lib/dates";
import { layerHref, layerOfFile, memoryFileQuery } from "~/lib/memory";
import { useOwner } from "~/lib/owner";
import { useParam, withParams } from "~/lib/params";
import { setView } from "~/lib/view";

const MODES: [string, string][] = [
  ["read", "読む"],
  ["write", "書く"],
  ["history", "経緯"],
];

/** The file's text, saved with a button and kept in its history as the owner's change. */
function Write(props: { path: string; file: FileContent }) {
  const [from, setFrom] = createSignal(props.file.text);
  const [text, setText] = createSignal(from());
  const dirty = () => text() !== from();
  /** The file was written elsewhere after this was begun. */
  const moved = () => dirty() && props.file.text !== from();
  const take = () => {
    setFrom(props.file.text);
    setText(props.file.text);
  };
  createEffect(
    on(
      () => props.file.text,
      () => !dirty() && take(),
      { defer: true },
    ),
  );
  const save = () =>
    act(async () => {
      const sent = text();
      const file = await api.memory.writeFile({ path: props.path, text: sent.replace(/\r\n/g, "\n") });
      // What is kept may not be what was sent: links are written out in full.
      setFrom(file.text);
      if (text() === sent) setText(file.text);
    }, "保存した");
  useBeforeLeave((e) => {
    if (!dirty() || e.defaultPrevented) return;
    e.preventDefault();
    void confirmed("書きかけを捨てる", "捨てる", `${props.path} に書いたものは、まだ保存していない。`).then((yes) => yes && e.retry(true));
  });
  return (
    <>
      <Show when={moved()}>
        <Callout
          tone="warn"
          live
          action={
            <Button small onClick={take}>
              書きかけを捨てて読み直す
            </Button>
          }
        >
          書いているあいだに、このファイルがほかで直された。保存すると、その直しの上に書く。
        </Callout>
      </Show>
      <TextArea
        class="mono memory-text"
        aria-label={`${props.path} を書く`}
        value={text()}
        onInput={(e) => setText(e.currentTarget.value)}
      />
      <div class="row">
        <span class="sub grow">保存すると、あなたが直したこととして経緯に残る。</span>
        <Button variant="primary" disabled={!dirty()} onClick={() => void save()}>
          保存
        </Button>
      </div>
    </>
  );
}

export default function MemoryFile() {
  const path = useParam("path");
  const asked = useParam("mode");
  const mode = () => asked() || "read";
  const file = createAsync(async () => (path() ? memoryFileQuery(path()) : undefined));
  const owner = useOwner();
  createEffect(() => setView("記憶", path()));

  return (
    <Show when={path()} fallback={<EmptyState>ファイルを選んでいない</EmptyState>}>
      <PageHeader
        back={{ href: layerHref(layerOfFile(path())), label: "層へ戻る" }}
        title={<span class="mono memory-path">{path()}</span>}
        actions={
          <SegmentedLinks
            label="見方"
            items={MODES.map(([m, text]) => [withParams("/memory/file", { path: path(), mode: m }), text, m === mode()])}
          />
        }
      />
      <Show when={file()} fallback={<Loading />}>
        {(f) => (
          <Switch
            fallback={
              <div class="card">
                <div class="rec-prose">
                  <Markdown text={f().text} attributes />
                </div>
              </div>
            }
          >
            <Match when={mode() === "write"}>
              <Show when={f().path} keyed>
                {(written) => <Write path={written} file={f()} />}
              </Show>
            </Match>
            <Match when={mode() === "history"}>
              <List>
                <For each={f().history} fallback={<EmptyState>まだ経緯はない</EmptyState>}>
                  {(c) => {
                    const aides = c.author === "Famulen";
                    return (
                      <ListRow
                        title={<span class="memory-change">{c.message}</span>}
                        sub={
                          <>
                            <Tag tone={aides ? "aide" : "said"}>{aides ? owner.aide() : "あなた"}</Tag>
                            <span>{dateTimeText(c.at, owner.tz())}</span>
                          </>
                        }
                      />
                    );
                  }}
                </For>
              </List>
            </Match>
          </Switch>
        )}
      </Show>
    </Show>
  );
}
