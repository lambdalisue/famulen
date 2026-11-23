import { createAsync, query } from "@solidjs/router";
import { For, Show } from "solid-js";
import { act } from "~/lib/act";
import { api } from "~/lib/api";
import { hrefOf } from "~/lib/records";
import { Button, List, Section, Tag } from "~/components/ui";

const checkQuery = query(async () => (await api.notebook.checkNotebook({})).findings, "notebookCheck");

/** Where the notebook strays from its rules, and the fix for what has one. */
export default function NotebookCheck() {
  const findings = createAsync(() => checkQuery());
  const href = (path: string) => {
    const m = /^records\/([^/]+)\/([^/]+)\.md$/.exec(path);
    return m ? hrefOf(m[1], m[2]) : undefined;
  };
  return (
    <Section title="手帳の書き方" id="notebook-check" note="手帳のファイルが決まりどおりに書かれているか。手で書き換えたものは、求めるまで直さない。「決まりどおりに直す」は、並びや日時の書き方のように、直し方が一つに決まるものだけを直す。">
      <Show when={(findings() ?? []).length > 0} fallback={<p class="sub">{findings() ? "決まりから外れているものはない。" : "確かめている"}</p>}>
        <List>
          <For each={findings()}>
            {(f) => (
              <div role="listitem" class="item">
                <Tag tone={f.error ? "warn" : undefined}>{f.error ? "誤り" : "注意"}</Tag>
                <span class="grow">
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
        </List>
        <div>
          <Button onClick={() => void act(() => api.notebook.fixNotebook({}), "直した")}>
            決まりどおりに直す
          </Button>
        </div>
      </Show>
    </Section>
  );
}
