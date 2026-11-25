import { createAsync } from "@solidjs/router";
import { createEffect, For, Show } from "solid-js";
import DreamNow from "~/components/DreamNow";
import Markdown from "~/components/Markdown";
import { EmptyState, Legend, LinkButton, List, Loading, PageHeader, Section, Tag } from "~/components/ui";
import type { Dream } from "~/gen/famulen/v1/memory_pb";
import { dateTimeText } from "~/lib/dates";
import { OP_LABELS } from "~/lib/dreams";
import * as icons from "~/lib/icons";
import { dreamSettingsQuery, dreamsQuery } from "~/lib/memory";
import { useOwner } from "~/lib/owner";
import { useParam } from "~/lib/params";
import { setView } from "~/lib/view";

const OPS: [string, string][] = [
  ["足した", "var(--op-added)"],
  ["直した", "var(--op-edited)"],
  ["置き換えた", "var(--op-replaced)"],
  ["忘れた", "var(--op-forgotten)"],
];

/** What a bar is made of, in the order of OPS. */
function opCounts(d: Dream): number[] {
  return [d.added, d.edited, d.replaced, d.archived];
}

/** Each night's changes as a bar, pressed to open the night. */
function Nights(props: { nights: Dream[]; at: number; tz: string }) {
  const max = () => Math.max(5, ...props.nights.map((n) => opCounts(n).reduce((a, b) => a + b, 0)));
  return (
    <div class="nights">
      <div class="nights-axis" aria-hidden="true">
        <span>{max()}</span>
        <span>0</span>
      </div>
      <div class="nights-bars" style={{ "--n": props.nights.length }}>
        <For each={props.nights}>
          {(n, i) => {
            const counts = opCounts(n);
            const said = `${dateTimeText(n.at, props.tz)} · ${OPS.map(([label], k) => `${label} ${counts[k]}`).join(" · ")}`;
            return (
              <a
                class="night"
                classList={{ on: i() === props.at }}
                href={`/memory?night=${n.id}`}
                title={said}
                aria-label={said}
                aria-current={i() === props.at ? "true" : undefined}
              >
                <For each={counts}>
                  {(c, k) => (
                    <Show when={c > 0}>
                      <span style={{ height: `${(c * 100) / max()}%`, background: OPS[k()][1] }} />
                    </Show>
                  )}
                </For>
              </a>
            );
          }}
        </For>
      </div>
    </div>
  );
}

/** What one night changed, and why. */
function Night(props: { night: Dream; title: string; before?: Dream; after?: Dream }) {
  const n = () => props.night;
  const href = (d: Dream) => `/memory?night=${d.id}`;
  const counts = (): [string, number][] => [
    ["足した", n().added],
    ["直した", n().edited],
    ["置き換えた", n().replaced],
    ["確かめた", n().confirmed],
    ["忘れた", n().archived],
    ["保留にした", n().pendingAdded],
    ["タスクにした", n().taskMade],
    ["古いかもの印", n().doubted],
  ];
  return (
    <Section
      title={props.title}
      note={n().status}
      action={
        <>
          <Show when={props.before}>{(d) => <LinkButton small icon={icons.chevronLeft} label="前の夢" href={href(d())} />}</Show>
          <Show when={props.after}>{(d) => <LinkButton small icon={icons.chevronRight} label="次の夢" href={href(d())} />}</Show>
        </>
      }
    >
      <dl class="figures">
        <For each={counts()}>
          {([label, count]) => (
            <div classList={{ none: count === 0 }}>
              <dd class="mono">{count}</dd>
              <dt class="sub">{label}</dt>
            </div>
          )}
        </For>
      </dl>
      <List>
        <For each={n().changes} fallback={<EmptyState>変更はなかった</EmptyState>}>
          {(c) => (
            <div class="item memory-change-row" role="listitem">
              <Tag>{OP_LABELS[c.op] ?? c.op}</Tag>
              <div class="grow">
                <Markdown inline text={c.text} />
                <div class="sub">
                  {c.fromFile ? `${c.fromFile} → ${c.file}` : c.file}
                  <Show when={c.reason}>
                    {" · "}
                    <Markdown inline text={c.reason} />
                  </Show>
                  <Show when={c.previous}>
                    {" · 前は「"}
                    <Markdown inline text={c.previous} />
                    {"」"}
                  </Show>
                </div>
              </div>
            </div>
          )}
        </For>
      </List>
      <p class="sub">
        記憶の変更は 1
        つのコミットにまとまる。全体の書き直しはせず、項目ごとの小さな変更だけを当てる。ファイルを半分より小さくする変更と、あなたの言ったことを消す変更は当てない。
        <Show when={n().commit}>
          {" "}
          <span class="mono">commit {n().commit.slice(0, 7)}</span>
        </Show>
      </p>
    </Section>
  );
}

export default function Dreams() {
  createEffect(() => setView("記憶", "夢"));
  const wanted = useParam("night");
  const dreams = createAsync(() => dreamsQuery());
  const plan = createAsync(() => dreamSettingsQuery());
  const owner = useOwner();
  const nights = () => dreams()?.dreams ?? [];
  const index = () => {
    const found = nights().findIndex((n) => n.id.toString() === wanted());
    return found >= 0 ? found : nights().length - 1;
  };
  const changed = () => nights().some((n) => opCounts(n).some((c) => c > 0));

  return (
    <>
      <PageHeader
        title="夢"
        note={
          <>
            次は {dreams()?.next}。<Show when={plan()?.earlyAt}>{(n) => `気づきが ${n()} 件たまれば早めに見る。`}</Show>
            いま待っている気づき {dreams()?.waitingNotes ?? 0} 件。
          </>
        }
        actions={<DreamNow linkWhy />}
      />
      <Show when={dreams()} fallback={<Loading />}>
        <Section title="夢ごとの変化（この 2 週間）">
          <Show when={nights().length > 0} fallback={<EmptyState>まだ夢を見ていない</EmptyState>}>
            <Show when={changed()} fallback={<EmptyState>この 2 週間、記憶は変わっていない</EmptyState>}>
              <Legend items={OPS.map(([text, background]) => ({ text, style: { background } }))} />
              <Nights nights={nights()} at={index()} tz={owner.tz()} />
              <p class="sub">棒を押すと、その夜の夢を下に開く。確かめただけの変更は数に入れない。</p>
            </Show>
          </Show>
        </Section>
        <Show when={nights()[index()]}>
          {(n) => (
            <Night
              night={n()}
              title={`${index() + 1 === nights().length ? "最後の夢 · " : ""}${dateTimeText(n().at, owner.tz())}`}
              before={nights()[index() - 1]}
              after={nights()[index() + 1]}
            />
          )}
        </Show>
      </Show>
    </>
  );
}
