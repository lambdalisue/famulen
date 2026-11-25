import { create } from "@bufbuild/protobuf";
import { createAsync } from "@solidjs/router";
import { ErrorBoundary, For, type JSX, Show, Suspense } from "solid-js";
import { EmptyState, ErrorState, IconButton, LinkButton, Loading } from "~/components/ui";
import { NoteOriginSchema } from "~/gen/famulen/v1/memory_pb";
import { say } from "~/lib/act";
import { dateTimeText } from "~/lib/dates";
import * as icons from "~/lib/icons";
import { noteOriginQuery } from "~/lib/memory";
import { useOwner } from "~/lib/owner";

const KINDS: Record<string, string> = { mail: "メール", slack: "Slack" };

/** The values that are known, each under what it is. */
export function Facts(props: { rows: [label: string, value: JSX.Element][] }) {
  return (
    <dl class="memory-working">
      <For each={props.rows.filter(([, value]) => !!value)}>
        {([label, value]) => (
          <>
            <dt>{label}</dt>
            <dd>{value}</dd>
          </>
        )}
      </For>
    </dl>
  );
}

/** An id or a path, to be copied. */
export function Copied(props: { text: string; what: string; href?: string }) {
  const copy = () => void navigator.clipboard?.writeText(props.text).then(() => say(`${props.what}を写した`));
  return (
    <span class="memory-copied">
      <Show when={props.href} fallback={<code>{props.text}</code>}>
        {(href) => (
          <a href={href()}>
            <code>{props.text}</code>
          </a>
        )}
      </Show>
      <IconButton plain icon={icons.copy} label={`${props.what}を写す`} size={14} onClick={copy} />
    </span>
  );
}

function Told(props: { id: string }) {
  const owner = useOwner();
  const origin = createAsync(async () => (props.id ? noteOriginQuery(props.id) : create(NoteOriginSchema)));
  return (
    <Show when={origin()} fallback={<Loading />}>
      {(o) => (
        <Show when={o().kind} fallback={<EmptyState>出どころは残っていない</EmptyState>}>
          <Facts
            rows={[
              ["どこ", [KINDS[o().kind] ?? o().kind, o().account].filter(Boolean).join(" · ")],
              ["チャンネル", o().channel],
              [o().kind === "mail" ? "差出人" : "書いた人", o().from],
              ["件名", o().subject],
              ["日時", o().at ? dateTimeText(o().at, owner.tz()) : ""],
              ["抜粋", o().excerpt],
              ["見張りの判断", [o().outcome, o().reason].filter((t, i, all) => t && all.indexOf(t) === i).join(" · ")],
            ]}
          />
          <Show when={o().url}>
            {(url) => (
              <LinkButton small icon={icons.externalLink} href={url()}>
                {o().kind === "mail" ? "メールを開く" : "発言を開く"}
              </LinkButton>
            )}
          </Show>
        </Show>
      )}
    </Show>
  );
}

/** Where a note, by its id, came from, or what a source a memory names is. */
export function Origin(props: { id: string }) {
  return (
    <ErrorBoundary fallback={(error) => <ErrorState error={error} />}>
      <Suspense fallback={<Loading />}>
        <Told id={props.id} />
      </Suspense>
    </ErrorBoundary>
  );
}

function Message(props: { source: string; note: string }) {
  const owner = useOwner();
  const origin = createAsync(() => noteOriginQuery(props.source));
  return (
    <Show when={origin()?.kind ? origin() : undefined} fallback={<code>{props.source + props.note}</code>}>
      {(o) => (
        <>
          <span>
            {[KINDS[o().kind] ?? o().kind, o().channel || o().account, o().from || o().subject, o().at ? dateTimeText(o().at, owner.tz()) : ""]
              .filter(Boolean)
              .join(" · ")}
            {props.note}
          </span>
          <Show when={o().url}>
            {(url) => (
              <LinkButton small icon={icons.externalLink} href={url()}>
                開く
              </LinkButton>
            )}
          </Show>
        </>
      )}
    </Show>
  );
}

/** A source as a memory writes it, with the way to it where it can be found. */
export function Source(props: { text: string }) {
  const named = () => /^(slack:[^/（(\s]+\/[\d.]+|(?:slack|mail)\/[^（(\s]+)\s*([（(].*)?$/.exec(props.text);
  return (
    <div class="memory-source">
      <Show when={named()} fallback={<span>{props.text}</span>}>
        {(m) => (
          <ErrorBoundary fallback={<code>{props.text}</code>}>
            <Suspense fallback={<code>{props.text}</code>}>
              <Message source={m()[1]} note={m()[2] ?? ""} />
            </Suspense>
          </ErrorBoundary>
        )}
      </Show>
    </div>
  );
}
