import { createSignal, For, Show } from "solid-js";
import { Icon } from "~/components/ui";
import type { Attachment } from "~/gen/famulen/v1/console_pb";
import { attachmentUrl, type Draft, sizeOf } from "~/lib/attachments";
import * as icons from "~/lib/icons";

/** How long the backend keeps a file. */
const KEPT_SECONDS = 30 * 24 * 60 * 60;

function Attached(props: { a: Attachment; at: bigint }) {
  const [missing, setMissing] = createSignal(false);
  const gone = () => missing() || Date.now() / 1000 - Number(props.at) > KEPT_SECONDS;
  const file = () => (
    <a class="attached-file" href={attachmentUrl(props.a.id)} download={props.a.name}>
      <Icon svg={icons.paperclip} size={14} />
      <span>{props.a.name}</span>
      <span class="sub">{gone() ? "もう残っていない" : sizeOf(Number(props.a.size))}</span>
    </a>
  );
  return (
    <Show
      when={!props.a.problem}
      fallback={
        <div class="attached-file missing">
          <Icon svg={icons.alertTriangle} size={14} />
          <span>{props.a.name}</span>
          <span>届かなかった：{props.a.problem}</span>
        </div>
      }
    >
      <Show when={props.a.id} fallback={<div class="attached-file">{props.a.name}（届くのを待っている…）</div>}>
        <Show when={props.a.mime.startsWith("image/") && !gone()} fallback={file()}>
          <a class="attached-image" href={attachmentUrl(props.a.id)} aria-label={props.a.name} target="_blank" rel="noreferrer">
            <img src={attachmentUrl(props.a.id)} alt={props.a.name} loading="lazy" onError={() => setMissing(true)} />
          </a>
        </Show>
      </Show>
    </Show>
  );
}

/** The files that came with a message. */
export function AttachedList(props: { files: Attachment[]; at: bigint }) {
  return (
    <Show when={props.files.length > 0}>
      <div class="attached" data-testid="attached">
        <For each={props.files}>{(a) => <Attached a={a} at={props.at} />}</For>
      </div>
    </Show>
  );
}

/** The files going along with what is being written. */
export function DraftList(props: { drafts: Draft[]; onRemove: (draft: Draft) => void }) {
  return (
    <Show when={props.drafts.length > 0}>
      <div class="drafts" aria-live="polite">
        <For each={props.drafts}>
          {(d) => (
            <div class="chip" classList={{ bad: !!d.problem }} data-testid="draft">
              <Icon svg={d.problem ? icons.alertTriangle : icons.paperclip} size={12} />
              <span>
                {d.name}
                {d.problem ? `：${d.problem}` : d.id ? "" : "（送っている…）"}
              </span>
              <button type="button" aria-label={`${d.name} を外す`} title="添えない" onClick={() => props.onRemove(d)}>
                <Icon svg={icons.x} size={12} />
              </button>
            </div>
          )}
        </For>
      </div>
    </Show>
  );
}
