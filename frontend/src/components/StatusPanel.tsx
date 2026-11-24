import { useAsked } from "~/components/ApprovalCard";
import { shortTime } from "~/lib/dates";
import { toJsonString } from "@bufbuild/protobuf";
import { createAsync, query } from "@solidjs/router";
import { createSignal, For, Show } from "solid-js";
import { createPages, PAGE, PageNav } from "~/components/logs/Pages";
import { InputItemSchema, type InputItem } from "~/gen/famulen/v1/console_pb";
import { api } from "~/lib/api";
import { statusQuery } from "~/lib/data";
import { useWaiting } from "~/lib/inbox";
import * as icons from "~/lib/icons";
import { routinesQuery } from "~/lib/records";
import { cancelWake, wakesQuery } from "~/lib/routines";
import { useOwner } from "~/lib/owner";
import { Button, EmptyState, IconButton, List, Tag, type Tone, whenConfirmed } from "~/components/ui";

const inputsQuery = query(async (before: number) => {
  const page = await api.console.listInputs({ before: BigInt(before), limit: PAGE });
  return { rows: page.inputs, more: page.more };
}, "inputs");

const INPUT_STATES: Record<string, [string, Tone | undefined]> = {
  pending: ["待ち", undefined],
  running: ["仕事中", "aide"],
  done: ["済", undefined],
  held: ["止まった", "warn"],
  failed: ["失敗", "danger"],
};

const SOURCES: Record<string, string> = {
  console: "コンソール",
  slack: "Slack",
  schedule: "定期の仕事",
  arrival: "サービスからの知らせ",
  wake: "起床予約",
  approval: "確認の答え",
  lookout: "見張り",
  worker: "委託",
};

const STATES: Record<string, string> = {
  running: "仕事をしている",
  waiting: "待っている",
  stopped: "止まっている",
};

/** Every input the secretary has, newest first, one JSON object a line. */
async function inputsAsJsonl(): Promise<string> {
  const lines: string[] = [];
  let before = 0n;
  for (;;) {
    const page = await api.console.listInputs({ before, limit: 200 });
    for (const input of page.inputs) lines.push(toJsonString(InputItemSchema, input));
    const last = page.inputs.at(-1);
    if (!page.more || !last) break;
    before = last.id;
  }
  return lines.map((l) => `${l}\n`).join("");
}

async function downloadInputs() {
  const blob = new Blob([await inputsAsJsonl()], { type: "application/jsonl" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = `famulen-inputs-${new Date().toISOString().replace(/[:.]/g, "-")}.jsonl`;
  a.click();
  URL.revokeObjectURL(url);
}

/** One input as a card: how far it got, where it came from, when, and what it said. */
function InputCard(props: { input: InputItem; tz: string }) {
  const [open, setOpen] = createSignal(false);
  const state = () => INPUT_STATES[props.input.status] ?? [props.input.status, undefined];
  return (
    <li class="status-input">
      <div class="status-input-head">
        <Tag tone={state()[1]}>{state()[0]}</Tag>
        <span class="status-input-from">
          {[SOURCES[props.input.source] ?? props.input.source, props.input.detail].filter(Boolean).join(" · ")}
        </span>
        <time class="mono status-input-at">{shortTime(props.input.at, props.tz)}</time>
      </div>
      <button type="button" class="status-input-text" classList={{ open: open() }} aria-expanded={open()} onClick={() => setOpen(!open())}>
        {props.input.text}
      </button>
    </li>
  );
}

/** What the secretary is doing, what waits for the owner and what comes next. */
export default function StatusPanel() {
  const status = createAsync(() => statusQuery());
  const routines = createAsync(() => routinesQuery());
  const wakes = createAsync(() => wakesQuery());
  const inputs = createPages(inputsQuery);
  const owner = useOwner();
  const tz = owner.tz;
  const upcoming = () =>
    (routines() ?? [])
      .filter((r) => r.enabled)
      .flatMap((r) =>
        r.runs
          .filter((run) => !run.skipped)
          .slice(0, 1)
          .map((run) => ({ id: r.id, title: r.title, when: r.when, at: run.at })),
      )
      .sort((a, b) => Number(a.at - b.at))
      .slice(0, 8);
  const { approvals, held, count: waiting } = useWaiting();
  const asked = useAsked();
  const [busy, setBusy] = createSignal(false);

  return (
    <div class="status-panel">
      <div class="status-now" data-state={status()?.state ?? ""}>
        <span class="status-badge" role="status">
          <span class="status-badge-dot" aria-hidden="true" />
          {STATES[status()?.state ?? ""] ?? status()?.state}
        </span>
        <Show when={status()?.doing}>
          <p class="status-doing">{status()?.doing}</p>
        </Show>
        <ul class="status-counts">
          <li class="status-count">
            <span>待っている入力</span>
            <b class="mono">{status()?.pendingInputs ?? 0}</b>
          </li>
          <li class="status-count" classList={{ warn: waiting() > 0 }}>
            <span>あなたを待っているもの</span>
            <b class="mono">{waiting()}</b>
          </li>
          <li class="status-count">
            <span>定期の仕事</span>
            <b class="mono">{routines()?.length ?? 0}</b>
          </li>
        </ul>
      </div>
      <section class="status-section">
        <h2>これからの予定</h2>
        <List>
          <For each={upcoming()}>
            {(r) => (
              <div role="listitem" class="item">
                <span class="mono sub when">{shortTime(r.at, tz())}</span>
                <span class="grow">{r.title}</span>
                <Tag>{r.when}</Tag>
              </div>
            )}
          </For>
          <For each={wakes()}>
            {(w) => (
              <div role="listitem" class="item">
                <span class="mono sub when">{shortTime(w.atUnix, tz())}</span>
                <span class="grow">
                  起床予約 · {w.note}
                  <Show when={w.why}>
                    <span class="sub">{w.why}</span>
                  </Show>
                </span>
                <IconButton
                  icon={icons.trash}
                  label={`起床予約「${w.note}」を取り消す`}
                  title="取り消す"
                  onClick={() => void whenConfirmed(`起床予約「${w.note}」を取り消す`, "取り消す", () => cancelWake(w.id))}
                />
              </div>
            )}
          </For>
          <Show when={upcoming().length === 0 && (wakes() ?? []).length === 0}>
            <EmptyState>なし</EmptyState>
          </Show>
        </List>
      </section>
      <section class="status-section">
        <h2>あなたを待っているもの</h2>
        <p class="sub">確かめてほしい道具と、途中で止まった頼みごと。答えるのは会話の欄から。</p>
        <List>
          <For each={approvals()}>
            {(a) => (
              <div role="listitem" class="item">
                <span class="mono sub when">{shortTime(a.createdAt, tz())}</span>
                <Tag tone="warn">確かめる</Tag>
                <span class="grow">
                  {asked(a.tool).does} <span class="sub">{asked(a.tool).where}</span>
                </span>
              </div>
            )}
          </For>
          <For each={held()}>
            {(h) => (
              <div role="listitem" class="item">
                <span class="mono sub when">{shortTime(h.at, tz())}</span>
                <Tag>止まった</Tag>
                <span class="grow">{[...h.text].slice(0, 90).join("")}</span>
              </div>
            )}
          </For>
          <Show when={waiting() === 0}>
            <EmptyState>なし</EmptyState>
          </Show>
        </List>
      </section>
      <section class="status-section">
        <div class="status-section-head">
          <h2>最近の入力</h2>
          <Button
            small
            icon={icons.download}
            disabled={busy()}
            onClick={() => {
              setBusy(true);
              void downloadInputs().finally(() => setBusy(false));
            }}
          >
            JSONL で書き出す
          </Button>
        </div>
        <ul class="status-inputs">
          <For each={inputs.rows()}>{(i) => <InputCard input={i} tz={tz()} />}</For>
        </ul>
        <Show when={inputs.loaded() && inputs.rows().length === 0}>
          <EmptyState>まだない</EmptyState>
        </Show>
        <PageNav pages={inputs} />
      </section>
    </div>
  );
}
