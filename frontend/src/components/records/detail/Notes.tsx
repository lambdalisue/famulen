import { useBeforeLeave } from "@solidjs/router";
import { createSignal, Show } from "solid-js";
import Markdown from "~/components/Markdown";
import { Button, confirmed, EmptyState } from "~/components/ui";
import type { RecordLink } from "~/gen/famulen/v1/records_pb";
import * as icons from "~/lib/icons";
import LinkTextarea from "../links/LinkTextarea";

/** Free Markdown: read as Arto draws it, written as plain text and saved with a button. */
export default function Notes(props: {
  text: string;
  label: string;
  empty: string;
  links: RecordLink[];
  onSave: (text: string) => Promise<boolean>;
}) {
  const [draft, setDraft] = createSignal<string | undefined>();
  const [preview, setPreview] = createSignal(false);
  const save = async () => {
    if (await props.onSave(draft() ?? "")) setDraft(undefined);
  };
  const dirty = () => draft() !== undefined && draft() !== props.text;
  const drop = () => confirmed("書きかけを捨てる", "捨てる", `${props.label}に書いたものは、まだ保存していない。`);
  const giveUp = async () => {
    if (!dirty() || (await drop())) setDraft(undefined);
  };
  useBeforeLeave((e) => {
    if (!dirty() || e.defaultPrevented) return;
    e.preventDefault();
    void drop().then((yes) => {
      if (!yes) return;
      setDraft(undefined);
      e.retry(true);
    });
  });
  return (
    <section class="rec-section rec-notes-section">
      <header class="rec-head">
        <h3 class="grow">{props.label}</h3>
        <Show
          when={draft() !== undefined}
          fallback={
            <Button small icon={icons.pencil} aria-label={`${props.label}を書く`} onClick={() => setDraft(props.text)}>
              書く
            </Button>
          }
        >
          <Button small variant="ghost" onClick={() => setPreview(!preview())}>
            {preview() ? "書く" : "プレビュー"}
          </Button>
          <Button small onClick={() => giveUp()}>
            やめる
          </Button>
          <Button small variant="primary" onClick={() => save()}>
            保存
          </Button>
        </Show>
      </header>
      <Show
        when={draft() === undefined || preview()}
        fallback={<LinkTextarea label={props.label} value={draft() ?? ""} onInput={setDraft} onSave={() => void save()} />}
      >
        <Show when={draft() ?? props.text} fallback={<EmptyState>{props.empty}</EmptyState>}>
          {(text) => (
            <div class="rec-prose">
              <Markdown text={text()} links={props.links} />
            </div>
          )}
        </Show>
      </Show>
    </section>
  );
}
